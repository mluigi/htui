# ANA-18 - Jev model implementability

> **Scope note:** Research whether the Jev model by TypeSafe AI is implementable for decisions in
> this project.
>
> **Status (2026-09-19): concluded.** Verdict: Not implementable.
>
> **Amended 2026-09-22 after review.** The verdict is revised to **not adopted now** and the
> reasoning under it is replaced: the original rested on a requirement mis-citation. The outcome for
> the backlog is unchanged - no `MOD-N` item is spawned. See §5 and §6.

---

## 1. Context and problem statement

TypeSafe AI's Jev is a specialized "System One" AI model designed for high-speed, structured
decision-making rather than generative text tasks. It is non-autoregressive and evaluates input
state to return structured answers (Choice, Score, Noul) in roughly 70-500 milliseconds. Because it
does not generate free-form text, output tokens are billed at zero; input is priced at $0.042 per
million tokens, against a headline claim of 193.6x faster and 444.6x cheaper than small frontier
models on System One tasks.

The question is whether this model can be implemented for decision points within `htui`'s
orchestrator.

**Naming.** The model is **Jev**, not `JEV`, and the vendor is **TypeSafe AI**, a San Francisco lab
that left stealth on 2026-09-15 with $40M in seed funding. It is unrelated to Typesafe/Lightbend of
Scala. The original draft of this document used `JEV` and `TypeSafe` throughout; both are corrected
here because a future reader greps the wrong vendor otherwise.

Sources: https://typesafe.ai/blog/introducing-system-one-models-and-jev,
https://www.latent.space/p/ainews-jev-a-system-one-model-that,
https://www.tomshardware.com/tech-industry/artificial-intelligence/typesafe-ais-jev-offers-an-alternative-to-llms-that-claims-to-be-193x-faster-and-445x-cheaper-system-one-type-model-is-bespoke-for-probabilistic-decision-making

## 2. Decision primitives in htui

Currently, `htui` contains the following major agentic decision paths:
1. **Fan-out selection (R-ORCH-7):** A judge evaluates candidate steps and selects a winner (see
   `docs/ANA-2.md` §4.5).
2. **Review-to-implement loop (R-ORCH-3):** A review phase produces a review document containing
   findings and a verdict (`request-changes` or `approve`).

Other routing and gating decisions are either deterministic logic or human-driven (e.g. gates
downgraded to `never` in auto mode, capability checks, overlap admission). That is not an accident
of the current implementation: `R-ID-6` ("No LLM agent is ever in a sync, cache, import or
bookkeeping path. Those are deterministic code", `docs/REQUIREMENTS.md:49-50`) and Invariant 3 ("No
agent in a bookkeeping path", `docs/ANA-2.md:114-118`) forbid putting a model there. Jev's cheapest
pitch - a fast router in front of every branch - is therefore closed by rule, independently of
anything about the model. This is the strongest argument against adoption in the repository and the
original draft did not use it.

## 3. The judge contract versus Jev

Per `docs/ANA-2.md` §4.5 (Fan-out selection and the judge contract):
- The judge is a configured agent that produces a document of kind `judge`.
- Invariant 3 states: "Enforced by the judge's output being an integer index **plus a rationale
  document**, never a status or a row" (`docs/ANA-2.md:117-118`).
- The judge's prompt instruction is: "return the winning `fanout_index` **and a one-line reason per
  candidate**" (`docs/ANA-2.md:836`).
- The output format is a fenced JSON block: `{ "winner": 2, "reasons": { "0": "...", "1": "...",
  "2": "..." } }` (`docs/ANA-2.md:846-852`).

The original draft concluded from this that Jev "fundamentally violates `htui`'s invariant that
every refusal/decision must leave a rationale a human can read (`R-ID-3`)". **That citation is
wrong.** `R-ID-3` is "Postgres is the single source of truth" (`docs/REQUIREMENTS.md:40-42`), a
persistence rule, not an explainability rule. Invariant 7 cites `R-ID-3` for refusals leaving "a row
a human can read" (`docs/ANA-2.md:132-134`) - again persistence. No requirement in
`docs/REQUIREMENTS.md` mandates human-readable rationale for a model decision. The rationale
obligation exists only at the level of ANA-2's judge contract, which this repository owns and may
amend.

Against the real contract, Jev holds up better than the draft claimed:

| Contract point | Jev |
|---|---|
| Integer index | Choice, cardinality up to 255, against `max_fan_out` default 4 |
| Rationale document | Score returns per-level probability and confidence per candidate; the `judge` document body becomes a calibrated score table rather than four prose lines |
| Unparseable JSON block is a judge failure | Structurally impossible - Jev cannot return a value outside the declared answer space |
| `winner` outside the surviving set is a judge failure | Structurally impossible, same reason |
| Two calls with candidate order reversed | Still required. Questions are evaluated independently, but candidate block order is part of the input state, so order sensitivity is untested rather than absent. At this price the second call is free |
| Judge failure escalates to human selection | Improves: low confidence becomes an escalation trigger, routed through the existing `awaiting_approval` branch with no new machinery |

Nothing in Invariant 3 requires the rationale to be prose, and `R-ORCH-7` itself asks only for "a
judge step using a configured agent". A calibrated score table is arguably more auditable than an
LLM judge's one-liner, which is generated after the pick and is known to confabulate.

**The review loop is a genuine exclusion.** `R-ORCH-3` loops a review document with findings back
into `implement`. Findings are free-form text that a human and the next implementer both read. Jev
cannot produce them. That half of the original argument stands unchanged.

## 4. The actual blockers

Three, none of which the original draft named:

1. **Transport.** `R-AGT-4` defines the agent registry transport as `acp` or `cli`
   (`docs/REQUIREMENTS.md:153-156`), and `R-AGT-1` defines an `AgentDriver` around sessions,
   streamed typed events, follow-ups and permission requests. Jev is an HTTP decision API: no
   session, no event stream, no tool calls, no usage stream. The judge is an ordinary `run_step`
   recording agent, model, usage, timing and prompt digest (`R-ORCH-11`), so wiring Jev in means a
   third transport, a driver, and an enum migration - not a `judge_model` string swap. This is the
   real cost.
2. **Input state budget, unmeasured.** The judge prompt carries, per candidate, the output document
   body, the diff stat and unified diff over `before_hash..after_hash`, `verify_outcome` and the
   tail of the verification output (`docs/ANA-2.md:828-836`). Jev prices input tokens, so it has a
   state budget, and nobody has measured it against a realistic judge prompt. This is the one
   unknown that could disqualify the model outright, and it is the question this analysis should
   have asked first.
3. **`R-ID-6` and Invariant 3** close every decision path other than the judge, as §2 sets out.

## 5. Verdict

**Not adopted now.**

Jev is compatible with the fan-out judge in principle, subject to an amendment of ANA-2 §4.5's
rationale format from prose reasons to a calibrated score table. It is incompatible with the review
loop, which needs generative findings, and it is barred by `R-ID-6` from the deterministic
bookkeeping paths where its speed and price would matter most.

What remains is the judge alone: two calls per fan-out point, at a cap of 4 candidates. Against that
volume the current judge's cost and latency are already negligible, so the saving does not pay for a
new agent transport plus a contract amendment plus an unmeasured input budget.

Revisit if fan-out volume or judge latency ever appears in a batch cost report, or if a second
System One use case arrives that is not barred by `R-ID-6`. The first step on revisit is measuring a
real judge prompt against Jev's state budget (§4.2).

No `MOD-N` item will be spawned.

## 6. Amendment record

| Date | Change |
|---|---|
| 2026-09-19 | Original conclusion: "not implementable", on the grounds that Jev cannot produce the mandatory rationale required by `R-ID-3` and ANA-2 Invariant 3 |
| 2026-09-22 | `R-ID-3` citation corrected - it is a persistence requirement, not an explainability one, and no requirement mandates model-decision rationale. Verdict revised to "not adopted now" on cost/seam grounds: agent transport (`R-AGT-4`), unmeasured input state budget, and `R-ID-6` closing the non-judge paths. Review-loop exclusion retained. Vendor and model name corrected to TypeSafe AI / Jev. Backlog outcome unchanged |
