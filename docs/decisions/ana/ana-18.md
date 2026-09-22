# ANA-18 - Research JEV model implementability (done, 2026-09-19)

Research whether the Jev model by TypeSafe AI is implementable for decisions in `htui`.

Jev is a "System One" AI model that only outputs structured primitives (Choice, Score, Noul) and does not generate free-form text.

Verdict is not adopted now. Amended 2026-09-22: the original verdict of "not implementable" rested on a mis-citation of `R-ID-3`, which is a persistence requirement ("Postgres is the single source of truth"), not an explainability one. No requirement mandates a human-readable rationale for a model decision; the obligation lives only in ANA-2 §4.5's judge contract, which this repo may amend. Jev fits the fan-out judge in principle - Choice covers `max_fan_out`, and Score's per-level probabilities replace the prose reasons - but does not pay for itself against three real blockers: `R-AGT-4` transport is `acp` or `cli` and Jev is an HTTP decision API, its input state budget against a real judge prompt is unmeasured, and `R-ID-6` bars a model from every other decision path. The review loop (`R-ORCH-3`) remains a genuine exclusion, since findings are free-form text.

No `MOD` item spawned.

See `docs/ANA-18.md` for the full analysis.
