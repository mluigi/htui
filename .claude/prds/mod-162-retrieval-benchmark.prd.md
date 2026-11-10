# MOD-162 - Retrieval benchmark in the repo; rerun at workspace scale

Origin: `docs/ANA-37.md` §5, §11, P5. Requirement: `R-STO-8`. Routed PRD by `/handoff-run` (C2, C3 soft,
C4), ultracode for the implement phase, accepted by the maintainer 2026-11-10. Framing, evidence and all
decisions below confirmed by the maintainer the same day ("as recommended").

## Problem

ANA-37's retrieval numbers cannot be reproduced. The harness, the 72 frozen queries (sha256 `56679bac…`),
the 12 trial tasks, the scratch Qdrant collection and the scratch Postgres database lived in `/tmp` of an
hr sandbox and are gone. So every change to knowledge search (chunking, fusion, the embedder, new point
types from MOD-159, MOD-112 (4), MOD-167) ships without a quality number, and the workspace-scale claim
that made Qdrant a required service (`docs/ANA-37.md` §5.10) has never been measured.

## Evidence

- Checked 2026-11-10: a whole-filesystem search finds no `queries.json`, `tasks.json` or `ana37*`; `/tmp`
  holds no benchmark. ANA-37 §11 predicted it: "An hr sandbox restart wipes `/tmp` and `~/.cache`; P5
  exists so the numbers can be reproduced."
- No quality measurement exists in the repo (ANA-37 §4.1): the live tests embed with `HashEmbedder`,
  "whose dense arm is noise" (`docs/decisions/mod/mod-50.md`); MOD-68's goldens pin vector equality.
- An open lead the harness is needed for: on questions, dense-only scored 12/20 against the shipped
  fusion's 10/20 (ANA-37 §5.2); not significant, not followable without a harness.
- The workspace-scale extrapolation is UNMEASURED (ANA-37 §5.10, §11); requiring Qdrant rests on the
  maintainer's decision, not a measurement at that scale.
- A waiting consumer: MOD-169 runs its memory measurements "on MOD-162's harness" with §5.4's method.
- Not available: real agent use of `search_concepts` (host database only; ANA-37 §11).

## Users

- **Primary**: the maintainer, judging knowledge-search changes (MOD-159 indexing, MOD-161 related items,
  any fusion change for questions) and the workspace-scale claim; MOD-169, reusing the harness for memory.
- **Not for**: end users of htui (nothing ships in the product), CI quality gating.

## Hypothesis

We believe **a reproducible, in-repo retrieval benchmark** will **turn search-quality decisions into
measured ones** for **the maintainer and MOD-169**.
We'll know we're right when **one documented command, on a box with a scratch Qdrant and Postgres,
rebuilds the corpus from a pinned commit, checks the frozen query hash, and reproduces ANA-37's headline:
the hybrid search puts at least 20 of 24 reworded duplicates in its top 5 and beats the best lexical
baseline significantly (exact McNemar p < 0.05), with dense-only and sparse-only reported beside it.**

## Success Metrics

| Metric | Target | How measured |
|---|---|---|
| Reproduction of the duplicate headline | S recall@5 ≥ 20/24; S vs best lexical baseline p < 0.05 | the harness report on the `d15e9fea` corpus |
| Report completeness | every §5.1 backend plus htui's no-Qdrant baseline; recall@1/5, MRR@10, Wilson 95%, exact McNemar, §5.4 AUC and recall at zero false alarms | report contents against this list |
| Query freeze | query file committed with its sha256 before the first backend run; the harness refuses a mismatched hash | git history order; a test |
| Reproducibility | a second run from a clean scratch Qdrant moves no class by more than ANA-37's observed tie noise (one query per class) | two runs, compared |
| Workspace corpus (M2) | builder produces points for items, requirements, documents, notes and transcripts of ≥2 projects from Postgres | fixture-seeded test here; real run on the host |

## Scope

**MVP** — M1: the harness rebuilt in the repo on htui's own markdown corpus at a pinned commit: corpus
builder, every §5.1 backend (S, S10, S-dense, S-sparse, L-and/-or and their -len variants, G, G-dens) plus
htui's real no-Qdrant baseline (substring on key and title, never run by ANA-37), the metrics and
statistics above, and a report. Search goes through the production search code, not a copy.

**Decisions (maintainer, 2026-11-10)**

- **Q1 Query set.** Re-author the 72 queries in the same four classes and counts (24 duplicates, 20
  questions, 16 identifiers, 12 no-target); keep the ~25 query texts quoted in `docs/ANA-37.md` §5.3
  verbatim; commit the set with its sha256 before any backend runs. Results are compared to ANA-37's
  within noise, not query for query.
- **Q2 Corpus version.** Built from the files at a pinned commit, default `d15e9fea` (ANA-37's exact
  corpus); the pin is overridable to run on today's tree.
- **Q3 Baselines.** Keep L and G as ANA-37 defined them; add htui's own no-Qdrant baseline.
- **Q4 Gate.** The harness compiles in the gate and its metric, statistics and corpus logic are
  unit-tested; the benchmark run itself is manual (needs the 133 MB model and a live Qdrant).
- **Q5 Workspace data.** M2's corpus builder reads items, requirements, documents, notes and transcripts
  directly from Postgres, ahead of MOD-112 (4), with the note and transcript point formats marked as
  models of what MOD-112 (4) will index. The multi-project run stays owed until the host database holds
  two or more projects with content. M3 stays blocked on MOD-112 (1): an htui agent's lexical arm is MCP
  read tools with a text filter, not grep over files; M3's design is deferred until then.

**Out of scope**

- Changing production search (fusion tuning, chunking, embedder) — this item measures; a fix is its own item.
- Alternative backends (pgvector, embedded indexes) — excluded by ANA-37's scoping.
- Failing the gate on quality numbers — runs are manual (Q4).
- MOD-169's memory measurements — it reuses this harness and owns them.
- Counting agents' real use of `search_concepts` — host data, not a benchmark question.

## Delivery Milestones

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Harness and reproduction | One command rebuilds the `d15e9fea` corpus, checks the frozen query hash, runs every backend through htui's real search and prints a report that reproduces ANA-37's duplicate headline | in-progress | `.claude/plans/mod-162-m1-retrieval-benchmark.plan.md` |
| 2 | Workspace corpus from Postgres | The harness builds its corpus from a workspace's Postgres rows (several projects, notes, transcripts) and carries cross-project duplicate and question queries; tested on seeded fixtures; the real run owed on the host once ≥2 projects hold content | pending | — |
| 3 | Agent trial at workspace scale | The A/B trial reruns on a corpus that does not fit one read, lexical arm = MCP read tools with a text filter; design deferred | pending (blocked on MOD-112 (1), M2) | — |

## Open Questions

- [ ] M2: which host projects make up the first multi-project corpus — TBD, needs validation on the host
  database (htui had one project at ANA-37).
- [ ] M3: number of runs per arm and the grading protocol beyond ANA-37 §5.6 — deferred with M3.

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Re-authored queries differ in difficulty from the lost set, so "reproduced" is ambiguous | Medium | Medium | Same classes and counts, §5.3 texts verbatim, freeze before any run; judge against the success thresholds, not ANA-37's exact cells |
| The query author knows the targets (ANA-37 §5.9) | High | Medium | Recorded as a threat in the report; duplicates avoid title words as before |
| hr sandbox Qdrant is slow (creates 3.5-19 s) and wipes `~/.cache` (the model) on restart | High | Low | The run is manual; the reference run may be owed on the host; unit tests need neither |
| Hybrid ties reorder between runs (34 of 72 top-10s differed in ANA-37) | High | Low | Report run-to-run deltas; the reproducibility metric allows one query per class |
| Corpus at `d15e9fea` cannot be rebuilt (commit unreachable in a clone) | Low | Medium | Pin overridable; check the commit is reachable before freezing |
| The harness drifts from production search as `vector.rs` changes | Medium | Medium | Link the production search path, never a copy; the harness compiles in the gate |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*
