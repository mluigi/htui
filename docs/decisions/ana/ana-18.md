# ANA-18 - Research JEV model implementability (done, 2026-09-19)

Research whether the JEV model by TypeSafe is implementable for decisions in `htui`.

The JEV model is a "System One" AI model that only outputs structured primitives (Choice, Score, Noul) and does not generate free-form text.
Since `htui`'s architecture mandates that all AI decisions (like the fan-out judge in `docs/ANA-2.md` §4.5) produce human-readable rationales, this model is incompatible with the system's core invariants (`R-ID-3`, `ANA-2` Invariant 3). 

Verdict is not implementable. No `MOD` item spawned.

See `docs/ANA-18.md` for the full analysis.
