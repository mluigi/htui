# ANA-18 - JEV model implementability

> **Scope note:** Research whether the new JEV model by TypeSafe is implementable for decisions in this project.
>
> **Status (2026-09-19): concluded.** Verdict: Not implementable.

---

## 1. Context and problem statement

TypeSafe's JEV model is a specialized "System One" AI model designed for high-speed, structured decision-making rather than generative text tasks. It is non-autoregressive and evaluates input state to return structured answers (Choice, Score, Noul) in roughly 70–500 milliseconds. Because it does not generate free-form text, output tokens are essentially free.

The question is whether this model can be implemented for decision points within `htui`'s orchestrator.

## 2. Decision primitives in htui

Currently, `htui` contains the following major agentic decision paths:
1. **Fan-out selection (R-ORCH-7):** A judge evaluates candidate steps and selects a winner (see `docs/ANA-2.md` §4.5).
2. **Review-to-implement loop (R-ORCH-3):** A review phase produces a review document containing findings and a verdict (`request-changes` or `approve`).

Other routing and gating decisions are either deterministic logic or human-driven (e.g., gates downgraded to `never` in auto mode, capability checks, overlap admission).

## 3. The Judge Contract vs JEV

Per `docs/ANA-2.md` §4.5 (Fan-out selection and the judge contract):
- The judge is a configured agent that produces a document of kind `judge`.
- Invariant 3 explicitly states: "Enforced by the judge's output being an integer index **plus a rationale document**".
- The judge's prompt instruction is: "return the winning fanout_index **and a one-line reason per candidate**".
- The output format is a JSON block: `{ "winner": 2, "reasons": { "0": "...", "1": "...", "2": "..." } }`.

Since the JEV model does not generate free-form text and cannot produce the mandatory rationale document (the `reasons` fields), it fundamentally violates `htui`'s invariant that every refusal/decision must leave a rationale a human can read (`R-ID-3`). 

The review-to-implement loop also requires generative text for the findings in the review document.

## 4. Verdict

**Not implementable.**

The JEV model is incompatible with `htui`'s design philosophy, which mandates that AI decisions (like the fan-out judge or reviewer) produce human-readable rationales (Invariant 3, `R-ID-3`). Using a System One model that only outputs a choice/score would result in silent decisions with no explainability, violating the orchestrator's core tenets.

No `MOD-N` item will be spawned.
