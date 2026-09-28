# ANA-21 - Per-model weights for agent assignment, derived from public sources (concluded, 2026-09-28)

Opened at MOD-4 milestone 4, OQ-7 (`docs/decisions/mod/mod-4.md:237-240`,
`.claude/plans/mod-4-orch-fanout.plan.md` D60, D71), to answer how each configured agent/model
gets a weight that MOD-36 uses to spread a phase's fan-out candidates across models. Addresses
`R-AGT-8`, `R-ORCH-7`. Analysis: `docs/ANA-21.md`. Source-by-source re-read:
`docs/ANA-21-recheck-2026-09-28.md`.

## Verdict

A **weight is a coarse ordinal tier** on a 0-100 integer scale, drawn from five tiers —
S = 95, A = 80, B = 65, C = 45, D = 25 — plus 0 for "never given an extra fan-out slot". Only the
order carries meaning; a value outside the six is accepted and compared as an integer. Coarse
tiers, not fine scores, because the public record does not resolve fine scores: *Coding Agents Have
Converged* (arXiv 2609.17394) shows exact McNemar tests separate **none** of the 29 adjacent
top-30 pairs on SWE-bench Verified, while a change of scaffold moves one model by up to 29.8
points against a 8.8-point spread for the whole top thirty.

Weights vary **by the phase's `name`**, with `"*"` as the fallback, reusing the only vocabulary the
tree already has rather than introducing a task-kind enum. **Cost stays out of the weight**: all
three seeded agents are `subscription`, so a price-derived weight is undefined for all of them, and
folds would silently reorder models on a dated price change. Quota stays a hard gate in the walk
(`R-AGT-7`, ANA-4).

The weight lives in **`agent.settings.weights` (JSONB)** — registry data under `R-AGT-5`, already
mirrored to the cache, with MOD-23's editor as the natural UI and no DDL. Resolution is
`models[model][phase]`, then `models[model]["*"]`, then `models["*"][phase]`, then `models["*"]["*"]`,
then 50. Weights resolved at `StartRun` freeze into the run's graph snapshot as
`SnapshotCandidate.weight: Option<u8>` with `#[serde(default)]`, so crash recovery re-selects
against the same values (`R-ORCH-11`); `GraphSnapshot::V` stays 1.

**Apportionment.** Slot 0 is `eligible[0]`, the first eligible candidate in priority order; the
remaining slots take the other eligible candidates sorted by weight descending, then by agent not
yet used, then by priority position. Pure function of its inputs — no randomness, no quota
utilisation, no clock — so it is crash-stable and pinnable by table tests.

## The re-read

The 2026-09-25 draft carried second-hand figures: the session's proxy had blocked most benchmark
and vendor pages. On 2026-09-28 every figure was re-read from its primary source, with **no page
blocked** (the four `WebFetch` failures in that pass were tool-side credit exhaustion, and all
sources resolved over `curl`). **No model crossed a tier** — the coarse scheme absorbed the
correction, which is the result the draft predicted. What changed was the basis.

Reproduced: Opus 5.5 59.6 AA / 61.62 Vals on Terminal-Bench 4.0 and 57.6 on AA Intelligence Index
v4.3.2; Fable 5.1 49.49 Vals and ECI 165.0; gemini-3.1-pro 46.10 on SEAL; every arXiv 2609.17394
figure; and both vendor price sheets.

Corrected: GPT-6 Sol 43.9 and GPT-5.6 Sol 39.9 (draft had 43 / 37); Gemini 3.7 Flash 6.06 on Vals
(draft had "11.2, source unclear"); Gemini 3.8 Flash reads 40.9 at v4.3.2, so the draft's "pre-v4.3,
not comparable" caveat no longer applies.

**Not reproducible — the important part.** The draft's Sonnet 5 basis ("AA 38 max / 28 medium") and
its Haiku 4.5 basis ("AA 15") do not exist: AA scores no Sonnet 5 variant and no Haiku 4.5 at all,
and neither appears on either Arena top thirty. Also gone: the SWE-ECI file the draft cited for
Fable 5.1 is not in the Epoch hub's 2026-09 export, and **Claude Opus 5.5 is absent from Epoch ECI
entirely**. Sonnet 5's conflicting Terminal-Bench figure was re-attributed from "one aggregator" to
Vals — a proper independent run — with Vals' own note that seven provider refusals on that row are
scored as failures.

## Maintainer decisions, 2026-09-28

1. **`R-AGT-8` amended** with: "When the phase fans out, the remaining candidate slots are
   apportioned across the other eligible candidates by their per-model weight (ANA-21)." Correct as
   written only because decision 2 held.
2. **Slot 0 stays `eligible[0]`**, so weights never override the maintainer's explicit priority
   order and `R-AGT-8` stays literally true for the lead candidate and for every `fan_out = 1`
   phase.
3. **Learned weights are spawned**, not left to the volume trigger — see `docs/decisions/ana/`
   entry index for the follow-up, ANA-24.
4. **`sonnet` is not seeded as a key.** It has no independent analysis-kind figure, so it resolves
   through the `"*"` row at 65 on both axes. The only option the public record supports, and where
   §5.6's own rule lands when the honest input is "no figure".
5. **`gemini-3.8-flash-high` coding tier stays 45.** Two independent Terminal-Bench 4.0 runs and
   the AA index now agree it is bottom-tier, but TB 4.0's own interval is wide at that score and
   the tier rule does not license a new tier below 45 on this evidence.
6. **Gemini 3.7 vs 3.8 Flash split by task kind.** 3.7 leads the analysis axis (ECI 157.72 vs
   157.13, Arena Code 1593 vs 1580, Arena Text 1488 vs 1492) and 3.8 leads the coding axis (Vals
   13.13 vs 6.06) — the reverse of the version ordering the seed assumes. Seeded 80/25 and 65/45.
7. **Unmeasured effort rows are flattened, not guessed.** Every `gemini-3.{8,7,6}-flash-{medium,low}`
   string takes its family's `-high` value verbatim. No source scores effort below `-high`, so the
   one-tier-per-step assumption had nothing to step down from and is withdrawn.
8. **`gemini-3.1-pro-low` drops one tier to 45/25** for the effort gap, because all three of its
   supporting figures (SEAL 46.10, Arena Text 1487, ECI 154.92) describe the un-suffixed model.
9. **Refresh is a source registry, not a hardcoded fetcher.** The maintainer asked for fetched
   weights rather than hardcoded ones, and the verdict now does that for the axis where it is
   lawful to. See below.
10. **Finding better sources is spawned** as ANA-23.

## The refresh, and what the licence actually allows

`weights.sources` is a registry: each source carries its axis, URL, licence, and two booleans —
`dynamic` (may htui fetch it) and `redistributable` (may its numbers be written into a seed).
Enabling a source is a data edit plus a parser, never a schema change, which is what keeps the
infrastructure extensible rather than hardcoded to one provider.

A 2026-09-28 probe of every candidate source settled the field. **Epoch is the only one both
directly fetchable and licensed for this use** — plain HTTP, CC BY 4.0, 268 scored models — so the
**analysis axis is fetched** and nothing hardcoded survives a refresh there. The **implement axis
has no dynamically-permissible source at all**: Artificial Analysis's API is internal-use only and
bars products that give model-selection guidance, which a weight map is by construction; Vals and
SEAL publish no API; Arena has no official results API; Harbor's tbench.ai — the official
Terminal-Bench 4.0 board and the only source found that publishes `reasoning_effort` per row — has
its results behind an undocumented tRPC endpoint and no licence grant, though its harness repo is
Apache-2.0. So implement-axis weights are the maintainer's, and each is **dated**: an entry whose
`date` is absent or unparseable resolves to **0**, not to its `weight`, so a stale coding weight
stops being offered for a slot instead of being quietly used. That turns §8's alias-drift
mitigation from advisory into load-bearing.

ANA-23 exists to close the implement-axis gap and to re-probe the blocked sources, since licences
and endpoints move.

## What MOD-36 inherits

The shape and resolution of §5.4, the snapshot field, and the apportionment rule of §5.5 as the
selector behind the unchanged `AgentSelector` seam, built in the run worker's `Kit` from the
snapshot. Seeds: `weights` in `crates/htui-core/seeds/agent_{claude,claude_cli,agy}.json` with
§5.7's values, plus a data-only pg migration setting `settings.weights` on the existing seeded
rows by name only where the key is absent (MOD-38 has reserved `0005`; coordinate the number).

Three things weights do **not** do, which are MOD-36's to own. Weights do not create candidates —
seeded graphs have no `phase_agent` rows, so MOD-36 must give the maintainer a way to list several
`(agent, model)` candidates per phase before a weighted selector has anything to spread across
(`R-ORCH-1` stays the only source of candidates). The judge is never asked of the selector, so
there is no judge weight; judge identity bias becomes real the moment models mix, and MOD-36's own
HANDOFF line already names it — strip agent identity from candidate material, and write an
`item_note` when the project judge's agent is also one of the group's. And mixed-family budgeting
stays as it is: every prompt uses `TokenEstimator::DEFAULT`, already the tightest, until some item
actually chooses the estimator per model (`TokenEstimator::for_agent` does not exist; the function
is `for_model` and no production code calls it).

## Commits

- `204c543` — re-read every benchmark figure from its primary source; four draft figures corrected,
  three seeded bases withdrawn, `docs/ANA-21-recheck-2026-09-28.md` added
- close-out commit — the ten maintainer decisions, the source registry, the R-AGT-8 amendment, the
  spawn of ANA-23 and ANA-24
