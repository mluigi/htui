# ANA-21 - Per-model weights for agent assignment, derived from public sources

> **Scope note:** Research how each configured agent/model gets a weight that MOD-36 uses to choose
> which models run a phase's fan-out candidates: the weight's meaning, whether it varies by task
> kind, whether cost enters it, where it lives, how it is refreshed, and the initial values for the
> seeded agents. Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`,
> `docs/REQUIREMENTS.md`, `docs/ANA-2.md` (§4.1 candidate chain, §4.5 judge contract, §7 walk) and
> `docs/ANA-4.md` (quota). Origin: MOD-4 milestone 4, OQ-7 (`docs/decisions/mod/mod-4.md:237-240`,
> `.claude/plans/mod-4-orch-fanout.plan.md` D60, D71).
>
> **Requirements addressed:** `R-AGT-8`, `R-ORCH-7`. Touched at their seams: `R-AGT-5` (weights
> are data, never code keyed on a name), `R-AGT-7` (quota stays a gate), `R-ORCH-1` (candidates in
> priority order), `R-ORCH-11` (the run records what was chosen), `R-ID-6` (no model in a
> bookkeeping path).
>
> **Status (2026-09-28): concluded.** §3.3 was re-read from primary sources — Artificial Analysis
> (Terminal-Bench 4.0 and Intelligence Index v4.3.2), Vals, the Epoch benchmark hub, the Scale SEAL
> board, the Arena mirror and both vendor pricing pages, all first-hand, with no page blocked. Four
> of the draft's figures did not survive that read and are corrected in §3.3. The maintainer settled
> every open call on 2026-09-28; §7 records the ten decisions. The verdict's shape is a coarse
> ordinal tier keyed on phase name, cost excluded, the weight living in `agent.settings.weights`, and
> a refresh that is a **source registry** rather than a hardcoded fetcher: Epoch (CC BY 4.0) is
> fetched for the analysis axis, the implement axis is maintained and dated with an undated entry
> resolving to 0, and two follow-up items are spawned (learned weights; better benchmark sources).
> No implementation code is written here; MOD-36 owns the implementation and inherits §5 and §6.
> Close-out: `docs/decisions/ana/ana-21.md`.

---

## 1. Context and problem statement

MOD-4 milestone 4 runs every candidate of a fan-out group on the one agent the walk selects
("rival sampling"). The maintainer's OQ-7 answer (2026-09-23) was that candidates must eventually
spread across agents by a per-model weight, so the judge compares different models on the same
task; MOD-36 implements the spreading and this analysis supplies the weights. The HANDOFF item gave
the shape by example ("Gemini Flash 30, Claude Opus 5.5 60") and asked for:

1. whether weights are per task kind (analysis, implement, review, judge) or global;
2. whether cost enters the weight or stays a quota concern;
3. the weight's schema and home (`agent` row, `app_setting`, or a new table);
4. a refresh method (manual, scripted from named sources, or learned from htui's own judge
   verdicts);
5. initial values for the seeded agents.

The survey behind this document ran four readers in parallel: the tree, public quality benchmarks,
published pricing and latency, and prior art on model routers and on learning weights from
preferences. A second pass on **2026-09-28** re-read every figure this document leans on from its
primary source, with the egress proxy in place and no page blocked. §3.3 carries that pass;
`docs/ANA-21-recheck-2026-09-28.md` carries the full per-source record of what reproduced and what
did not. The verdict does not depend on any single number (it deliberately uses coarse tiers,
§5.1), and the coarse tiering absorbed the re-read: **no model changed tier as a result of it.**
What the re-read did change is the *stated basis* for three seeded values, which are now
under-determined and are the subject of the new §7 items.

## 2. Surface as read

Facts from the tree at `d966413`, which the verdict has to fit.

### 2.1 What the selector chooses among

- A model is **not a row**. It is free `TEXT` in `agent.models TEXT[]`, `agent.default_model`,
  `phase_agent.model`, `run_step.model` and `step_graph_phase.judge_model`
  (`crates/htui-store/migrations/0001_init.sql:94-106`, `:254-260`; `0003_orchestration.sql:17-27`).
- The unit `AgentSelector::select` picks is a `SnapshotCandidate { agent_id, agent_name, model }`
  (`crates/htui-core/src/model/run.rs:540-547`), which is one `phase_agent` row. `phase_agent`'s key
  is `(phase_id, position)`, so one agent can appear several times with different models. **A weight
  keyed on `(agent, model)` lines up exactly with what the selector chooses.**
- Model strings are per-installation vocabulary. The seeded `claude` and `claude-cli` rows have an
  empty `models` list and no default (`crates/htui-core/src/model/agent.rs:119-123`); fixtures and
  tests use the aliases `opus`, `sonnet`, `default`. `agy` carries eleven live-captured strings
  whose suffix is the effort level (`crates/htui-core/seeds/agent_agy.json`):
  `gemini-3.{8,7,6}-flash-{high,medium,low}`, `gemini-pro-agent`, `gemini-3.1-pro-low`, default
  `gemini-3.7-flash-high`. Rust code must never key on these strings (`agent.rs:128-129`, the
  `tests/extensibility.rs` vendor sweep, `R-AGT-5`).

### 2.2 The seam MOD-4 left

- `AgentSelector::select(phase, eligible, fanout_index)` and `FirstCandidate` live at
  `crates/htui-orch/src/engine.rs:129-171` (the ":82-106" range in HANDOFF is stale). Eligibility is
  the walk's (`crates/htui-orch/src/select.rs:139-186`: no row, quota, inline approval, not ready,
  budget); choice is the selector's.
- The selector is a static generic built by the run worker per task
  (`crates/htui/src/run_worker.rs:1186-1199`). It receives no agent rows, no quota and no weights.
- **The selector must be deterministic in `(phase, eligible, fanout_index)`.** `admit_indices`
  selects every index before writing any row, and `drive_group` re-runs it for missing indices after
  a crash (`engine.rs:2741-2789`). A random weighted draw would put a recovered index on a different
  agent than its siblings were planned against.
- Only rung 1 of the candidate chain (explicit `phase_agent` rows) yields more than one candidate;
  rungs 2 and 3 yield exactly one (`crates/htui-orch/src/graph.rs:588-685`). **The seeded graphs
  have no `phase_agent` rows** and no UI edits them, so on a seeded project a weighted selector has
  nothing to spread across.
- `review` never fans out (plan D64, `graph.rs:158-163`) and `local` isolation cannot fan out.
- The judge is never asked of the selector: its agent is `project.settings.judge_agent_id`, its
  model that agent's default (`graph.rs:572-583`, `crates/htui-orch/src/fanout.rs:307-321`);
  `step_graph_phase.judge_agent_id`/`judge_model` are unread.

### 2.3 Task kind

There is **no task-kind enum**. Phases carry a free `name`; the seeded values are `research`,
`verdict` (analysis), `prd`, `plan`, `implement`, `review` (feature), `reproduce`, `fix` (bug)
(`crates/htui-core/src/seed.rs:52-160`). The engine's only name-keyed rule is
`REVIEW_PHASE = "review"` (`engine.rs:69-75`).

### 2.4 Cost and quota today

- `quota::available` is a binary gate (exhausted, a full window, a status outside
  `allowed`/`allowed_warning`, spend at cap) and never ranks by utilization
  (`crates/htui-core/src/model/quota.rs:349-458`).
- `billing` plays no part in selection. All three seeded agents are `subscription`.
- `cost_micros` is a client-side estimate reported only by `claude`; `agy_acp_server` reports no
  usage at all (`crates/htui-core/src/prompt/estimate.rs:62-66`). `per_token_cap_run` is enforced by
  the recorder and by walk rules 2 and 5.

### 2.5 What a judge verdict leaves behind

Candidate `run_step` rows carry `agent_id`, `model`, `selected`, `verify_outcome` and `usage`; the
judge row (`fanout_index = -1`) carries its own `agent_id`/`model`; two `judge` documents hold
`{"winner": i, "reasons": {...}}` with no scores (`fanout.rs:120-141`). **Winner → model and judge →
model are recoverable from existing rows**, so a later learning step needs no new log. But no such
data exists yet: `FirstCandidate` never mixes models, and every production judge fails until MOD-11
(HANDOFF status line). Judge inputs carry no agent or model (`crates/htui-core/src/prompt/mod.rs:179-192`),
but nothing strips identity that leaks through the content, such as a `Co-Authored-By: Claude …`
trailer in a candidate's diff, and nothing prevents the judge sharing a model with a candidate.

### 2.6 Token estimation

`TokenEstimator::for_agent` does not exist. The function is `TokenEstimator::for_model`
(`estimate.rs:77-94`) and **no production code calls it**: the phase prompt, the judge prompt and
the preview hard-code `TokenEstimator::DEFAULT` (`engine.rs:4323`, `:4945`,
`crates/htui/src/preview.rs:252`). `DEFAULT` over-estimates for Gemini, so a mixed group is already
budgeted to the tightest family by accident. MOD-36's "mixed-family budgeting" open point is moot
until an estimator is actually chosen per model.

## 3. Public signals surveyed

### 3.1 The resolution problem

*Coding Agents Have Converged* (Liu et al., arXiv 2609.17394, Sept 2026) audits 254 SWE-bench
submissions from their published per-instance results. On Verified the top two entries each resolve
396/500; the top ten share 285 successes and 51 failures, so only 164 instances discriminate among
them; exact McNemar tests separate **none** of the 29 adjacent top-30 pairs. Solved sets are nested
(median 0.935 against 0.774 expected), and changing the scaffold moves one model by up to 29.8
points, more than the 8.8-point spread of the whole top 30. The authors conclude the score "belongs
to the model-scaffold pair" and should be reported as tiers. Preference data says the same: in the
2026-09-25 Arena Text snapshot all top-20 models sit between 1482 and 1506 Elo with overlapping CIs.
Terminal-Bench 4.0, the most-cited live agentic benchmark, has 66 tasks, so its 95% interval near
50% is roughly ±12 points.

Two consequences shape the verdict. Fine 0-100 numbers claim more resolution than any public source
has, so weights are **coarse tiers**. And htui runs every model inside a harness (Claude Code, the
`agy` ACP server) that is itself a scaffold, so a public number is at best a prior for htui's
`(agent, model)` pair.

### 3.2 Sources by task kind

| Task kind | Usable independent sources | Not usable | Notes |
|---|---|---|---|
| Implement | Terminal-Bench 4.0 as run by Artificial Analysis and Vals (fixed Terminus 2 harness); SWE-bench Pro standardized (Scale SEAL); SWE-rebench (fixed scaffold, fresh issues) | SWE-bench Verified (saturated at 96-97%, contamination found, archived by Vals 2026-09-01); Aider polyglot (unmaintained, no 2026 models); LiveCodeBench (saturated, algorithmic) | Vendor SWE-bench Pro (up to 89.9%) and SEAL's standardized run (top 61.5%, re-read 2026-09-28) are different measurements and never share a scale. Terminal-Bench versions are not comparable (Gemini 3.8 Flash: 87.6% on 2.1, 19.7% on 4.0), and the Epoch hub's own Terminal-Bench export is a 2.x file, not 4.0. **SEAL is stale against the current frontier** — newest Anthropic row Opus 4.6, newest Google row Gemini 3.1 Pro — so it can place only one seeded model, and the 2026-09-28 drop of SWE-ECI from the Epoch hub leaves the IRT-over-SWE option unavailable for now |
| Analysis | Artificial Analysis Intelligence Index (same version only); Epoch ECI; HLE as run by AA | GPQA Diamond (saturated, floor filter only) | The AA index is rescaled between versions (GPT-6 Astra 61 → 52.7 at v4.3.2); every stored value needs its version. **Coverage is the binding constraint, not availability**: at v4.3.2 the index scores no Sonnet 5 variant, no Haiku 4.5 and no Gemini 3.7 Flash or 3.1 Pro, and Epoch's ECI has no Opus 5.5 at all |
| Review | None at model level | Martian Code Review Bench ranks products, not models; CodeReviewBench is 30 PRs on dated models | Derive from implement and analysis |
| Judge | None current | JudgeBench and RewardBench 2 have no current frontier entries; a June 2026 study of 21 judges found rankings shift up to 14 places across benchmarks (arXiv 2606.19544) | Self- and same-family preference is documented (+10 to +25 points self, +3.4 to +8.4 same family) |

Licences matter for anything htui ships, and a 2026-09-28 probe of every candidate settled which
ones are usable:

| Source | Reachable | Machine-readable | Licence | Verdict |
|---|---|---|---|---|
| **Epoch** benchmark hub | yes, plain HTTP zip | yes, `eci_scores.csv`, 268 models | **CC BY 4.0** (fitting code MIT) | **the only source that may be both fetched and shipped**, with attribution |
| Harbor / **tbench.ai** (official TB 4.0) | yes | no — tRPC endpoint, no static JSON (`/api/leaderboard`, `/trpc/…`, `/leaderboard.json` all 404) | code Apache-2.0; **results unlicensed** | blocked, and it is the prize: the only effort-separated board found |
| Artificial Analysis | yes | API exists | internal-use only; **bars products giving model-selection guidance** | numbers citable here, never fetched into or shipped with htui |
| Vals | yes | no API, HTML only | not granted | blocked |
| Scale SEAL | yes | no API, HTML only | not granted | blocked, and stale (newest Anthropic row Opus 4.6) |
| Arena | yes | `arena-rank` is Apache-2.0 *code* | no official results API | the GitHub mirror used in §3.3 is a third-party scrape and is not a dependency |
| SWE-bench/experiments | yes | yes | **no licence declared** | moot: Verified is archived and saturated |
| LiveBench | yes | repo live (daily) | NOASSERTION | general capability only, not agentic coding |
| SWE-rebench | yes | no public API found | not granted | blocked |

A weight map is model-selection guidance by construction, which is what puts AA out of scope no
matter how convenient its API would be. Everything else is blocked on endpoint or licence, not on
principle — which is why the source registry in §5.4 exists and why finding a better source is a
separate item.

### 3.3 Independent figures behind the initial tiers

Every figure below was re-read from its primary source on **2026-09-28**; nothing here is
second-hand. Where a figure the 2026-09-25 draft carried did not reproduce, that is stated rather
than quietly dropped. Access record: `docs/ANA-21-recheck-2026-09-28.md`.

**Terminal-Bench 4.0** — two independent runs of the same 66-task set. Artificial Analysis (page
payload) and Vals (`https://www.vals.ai/benchmarks/terminal-bench-4`, updated 2026-09-22; Terminus
2 harness, avg@3, 8-hour limit per task, no partial credit):

| Model | Artificial Analysis | Vals |
|---|---|---|
| Claude Opus 5.5 | 59.6 (xhigh 59.6, high 56.6) | 61.62 |
| GPT-6 Astra | 59.1 (xhigh 59.6, high 54.0) | 57.07 |
| Claude Fable 5.1 | 52.0 (xhigh 55.1) | 49.49 |
| Claude Opus 5 | 49.0 | 45.45 |
| GPT-6 Sol / GPT-5.6 Sol | 43.9 / 39.9 | — / 27.78 |
| Gemini 3.8 Flash | 19.7 | 13.13 |
| Claude Sonnet 5 | **no row** | 8.08 |
| Gemini 3.7 Flash | **no row** | 6.06 |
| Claude Haiku 4.5 | **no row** | **no row** |

Two things this read settles. **Sonnet 5's 8.08 is a Vals figure, not an aggregator's.** Vals
itemises the row and records that seven of its task attempts were provider refusals scored as
failures, so the number understates the model by an unknown amount; the same page records
provider-side fallback on four Anthropic rows, and counting Opus 5.5's 30 fallback-served attempts
as failures moves it from 61.62 to 53.54, behind Astra. **And neither run separates Gemini 3.7 from
3.8 Flash the way the version numbers suggest**: Vals scores 3.7 at 6.06, *below* 3.8's 13.13.

**Artificial Analysis Intelligence Index v4.3.2** (the page title confirms v4.3.2). Claude Opus 5.5
57.6 (xhigh 56.0, high 53.6, medium 51.2); Claude Fable 5.1 53.4; GPT-6 Astra 52.7; Claude Opus 5
50.8; GPT-6 Sol 47.5; GPT-5.6 Sol 47.0; **Gemini 3.8 Flash 40.9**. No scored record exists for any
Claude Sonnet 5 variant, for Claude Haiku 4.5, or for any Gemini 3.7 Flash or Gemini 3.1 Pro
variant on this index — those slugs appear in the page only as unscored release entries.

**Epoch Capabilities Index** (`https://epoch.ai/data/benchmark_data.zip`, `eci_scores.csv`, 268
scored models, CC BY 4.0). GPT-6 Astra 166.6; Claude Fable 5.1 165.0; Claude Opus 5 162.67;
GPT-5.6 Sol 161.99; Gemini 3.7 Flash 157.72; Gemini 3.8 Flash 157.13; **Claude Sonnet 5 156.34
[153.45, 159.15]**; Gemini 3.1 Pro 154.92; **Claude Haiku 4.5 142.42**. **Claude Opus 5.5 is not
in the dataset at all**, so the index cannot separate the frontier coding leader from Fable 5.1.
The hub also ships no SWE-ECI file, so the draft's "SWE-ECI 167" for Fable 5.1 has no referent in
the current export. Its `terminalbench_external.csv` is a Terminal-Bench 2.x export (top score
0.847), not 4.0 — the version trap of §3.2, confirmed first-hand.

**SWE-bench Pro, Scale SEAL** (`https://labs.scale.com/leaderboard/swe_bench_pro_public`). Muse
Spark 1.1 61.50 ± 3.10; gpt-5.4 (xHigh) 59.10 ± 3.56; gemini-3.1-pro (thinking) 46.10 ± 3.60;
Claude Sonnet 4.5 43.60. The board is **stale against every model this verdict seeds**: its newest
Anthropic row is Opus 4.6 and its newest Google row is Gemini 3.1 Pro. It places no Opus 5.x, no
Sonnet 5 and no Gemini 3.7/3.8 Flash, so it can only ever justify the `gemini-3.1-pro-low` row.

**Arena** (GitHub mirror read 2026-09-28; leaderboards last updated 2026-09-25). Code: Claude Opus
5.5 max 1827 ± 18; GPT-6 Astra max 1792 ± 11; Claude Fable 5.1 max 1751 ± 10; Gemini 3.7 Flash
high 1593 ± 8; Gemini 3.8 Flash high 1580 ± 8. Text: the top twenty span 1476–1509; Claude Opus 5.5
high 1509 ± 12; Gemini 3.8 Flash high 1492 ± 5; Gemini 3.7 Flash high 1488 ± 5; Gemini 3.1 Pro
preview 1487 ± 3. Neither board's top thirty contains Claude Sonnet 5 or Claude Haiku 4.5.

Four corrections this read forces, none of which moves a model across a tier:

1. **Sonnet 5's draft basis does not exist.** The draft's "AA 38 at max, 28 at medium" is not
   reproducible — AA scores no Sonnet 5 variant, and Arena does not place it. The only current
   first-hand figure is Vals' 8.08, confounded by refusals. Sonnet 5 has **no independent
   analysis-kind figure at all**.
2. **Haiku 4.5's draft basis does not exist either.** The draft's "AA 15" is not reproducible —
   Haiku 4.5 is absent from AA entirely. Epoch ECI 142.42 is its only current figure.
3. **Gemini 3.8 Flash is comparable on the AA index after all.** The draft set it aside as "59 at
   launch, pre-v4.3, not comparable". At v4.3.2 it reads 40.9, in the bottom third — which agrees
   with its Terminal-Bench 4.0 placement and cuts against the draft's use of Arena Text alone to
   keep it in the analysis tier.
4. **Effort levels are separable on one source only.** Arena carries `-high` Gemini rows and no
   `-medium` or `-low` rows; AA, Vals, Epoch and SEAL score un-suffixed models. The
   `-medium`, `-low` and `gemini-3.6-flash-*` strings in the seed therefore have **no independent
   figure at all**, and §5.6 step 3's "one tier below" has nothing to anchor it.

The per-task-kind gap the verdict was built on survives, and hardens. Gemini Flash is bottom of
both Terminal-Bench 4.0 runs and of the AA index, yet sits inside the Arena Text cluster (1492,
1488) and inside Epoch's frontier ECI band (157.13, 157.72). **Gemini 3.7 Flash outranks Gemini 3.8
Flash on every non-Terminal-Bench source checked** — ECI, Arena Code and Arena Text — the opposite
of the version ordering the seed assumes, and the seed's default model is the 3.7 string. A single
global weight would misplace both families.

### 3.4 Prices

Per 1M tokens, input/output, all first-hand on 2026-09-28. Anthropic: Opus 5.5 4/20, Opus 5 5/25,
Fable 5.1 10/50, Sonnet 5 2/10, Haiku 4.5 1/5. Gemini: 3.7 Flash, 3.8 Flash **and 3.6 Flash** all
0.75/3.75 until 2026-12-31, then 1.50/7.50; Gemini 3.1 Pro 2/12 (≤200K), 4/18 above it. GPT-6 Sol
2/10 is the one price here not read from its vendor page; `openai.com` was not fetched in this pass
and the figure stays second-hand.

Two pricing facts sharpen the §4.4 argument. Anthropic's page now states that Sonnet 5's 2/10 was
launch pricing and that **the scheduled rise to 3/15 on 2026-09-01 will not occur**, so a price
that was once a dated promotion is now permanent — exactly the kind of silent reorder §4.4 says a
weight must not inherit. And the Gemini discount covers three model families, not two, so the
2027-01-01 step hits every `agy` string at once rather than a subset. Thinking is billed as output
everywhere and cannot be turned off on Opus 5.5 or Fable 5.x, so cost per task depends on effort far
more than on list price. All three vendors express subscription quota as an opaque 5-hour window
plus a weekly cap, never as tokens or dollars. Machine-readable price feeds exist (models.dev
`api.json` and LiteLLM's cost map, both MIT; OpenRouter `/api/v1/models`), but htui does not need
one under this verdict (§5.3).

## 4. Options considered

### 4.1 What a weight means

| Option | Meaning | Precedent | Fit for htui |
|---|---|---|---|
| **A. Traffic share** | Normalised probability; each slot is an independent weighted draw | LiteLLM `weight`, Portkey `loadbalance` | Breaks crash-determinism (§2.2) unless the draw is seeded; at small N it wastes diversity (weights 0.5/0.3/0.2 and N=3 give three slots to one model 16% of the time) |
| **B. Ordinal tier, deterministic apportionment** | Coarse quality tier; slots filled by descending tier, distinct candidates first | Tiered reporting (2609.17394); Claude Code/Aider pick models by role, not by lottery | Deterministic, testable, honest about resolution |
| **C. Learned posterior** | P(profile is best) from a Bayesian Bradley-Terry fit over judge verdicts, Thompson-sampled per phase | LMArena (BT with bootstrap CIs), RouteLLM, LiteLLM adaptive pools | No data exists yet (§2.5); needs ~100 verdicts per pair to separate 60/40 from a coin flip; judge self-preference is as large as the signal |

### 4.2 Per task kind or global

Global is simpler but contradicted by the data (§3.3). A fixed enum of four kinds would be a new
concept the tree does not have (§2.3) and would need a name → kind mapping somewhere in code. Keying
by **phase name with a fallback** reuses the only vocabulary that exists, keeps the mapping in data,
and a custom graph with new phase names falls back cleanly.

### 4.3 Where it lives

| Home | For | Against |
|---|---|---|
| `agent.settings.weights` (JSONB) | Registry data (`R-AGT-5`); already mirrored to the cache; MOD-23's editor is the natural UI; no DDL | Initial values in `seeds/*.json` reach only fresh databases (the seed tops up by name), so existing ones need a data migration |
| `app_setting` key | One row, seeded by migration | Must key on `agent.name` (ids are per database) and breaks on rename; `SettingKey` is scalar-only, so no editor; not mirrored |
| New table `model_weight(agent_id, model, phase_name, weight, source, …)` | Typed, FK-safe, provenance columns | A pg migration plus a cache migration plus `WriteStore` methods on every implementor, for a handful of integers |
| `phase_agent.weight` column | Per-phase for free | Seeded graphs have no `phase_agent` rows; duplicates the weight per graph |

### 4.4 Cost

Folding price into the weight (quality ÷ price, or inverse-square price as OpenRouter does between
providers of one model) is undefined for subscription agents, which are all three seeds, and would
silently reorder models on dated price changes (Gemini Flash doubles on 2027-01-01). Every router
surveyed that serves different models keeps cost as a separate knob (RouteLLM's threshold, Martian's
willingness-to-pay) or a hard constraint.

### 4.5 Refresh

Manual; scripted from public feeds; or learned from htui's verdicts. A scripted fetch runs into the
licence limits of §3.2, into brittle name mapping between sources and htui's per-installation model
strings (§2.1), and into benchmark version churn. Learning has no data yet and a known bias loop.

**Resolved 2026-09-28: all three, split by axis.** The maintainer asked for fetched weights rather
than hardcoded ones, and the 2026-09-28 probe (§5.6) found that the licence limit is narrower than
the draft assumed — one source, Epoch, is both directly fetchable and CC BY 4.0, so the analysis
axis is genuinely fetched. It also found the opposite limit the draft had not hit: the coding axis
has *no* dynamically-permissible source at all, so it stays maintained rather than manual-in-name-
only, and its entries are dated so staleness resolves to 0 instead of to a wrong number. Learning
is tracked by its own item. The brittle name mapping and version churn are handled by the source
registry rather than solved.

## 5. Verdict

### 5.1 A weight is a coarse ordinal tier

A weight is an integer on the 0-100 scale, drawn from five tiers: **S = 95, A = 80, B = 65, C = 45,
D = 25**, plus **0 = never given an extra fan-out slot**. Only the order matters: two candidates in
the same tier are equal, and the gap between 95 and 80 means nothing more than "ranks above". The
scale keeps the maintainer's 0-100 shape and ANA-5 §4.5's precedent of 0-100 integer tiers with
deterministic tie-breaks, while claiming no more resolution than §3.1 allows. A value that is not
one of the six is accepted and compared as an integer; the tiers are a convention for seeding and
display, not a CHECK.

### 5.2 Per phase name, with a fallback

Weights vary by the phase's `name`, with `"*"` as the fallback for any phase name not listed. The
seeds list `implement`, `fix` and `reproduce` separately where the coding tier differs from the
analysis tier, and let `research`, `verdict`, `prd`, `plan` and `review` fall to `"*"`. There is no
judge weight: the judge is not chosen by the selector (§2.2). Judge choice is covered by the
hardening in §6 instead.

### 5.3 Cost stays out of the weight

Quota and caps stay what they are: hard gates in the walk (`R-AGT-7`, ANA-4). The weight is quality
only. A soft cost preference, if one is ever wanted, is a separate per-token-only tie-break inside a
tier, and it would need a price feed with `valid_from`/`valid_until`; nothing in this verdict needs
one.

### 5.4 Home and shape: `agent.settings.weights`

```jsonc
// agent.settings.weights — registry data, R-AGT-5
{
  "basis": { "source": "ANA-21", "date": "2026-09-28" },   // provenance of the whole map

  // the scoring-source registry (§5.6). Extensible: adding a source is data,
  // not code. `dynamic` says whether htui fetches it; `redistributable` says
  // whether its numbers may be written into a seed at all.
  "sources": [
    { "id": "epoch-eci", "axis": "analysis",
      "url": "https://epoch.ai/data/benchmark_data.zip",
      "licence": "CC BY 4.0", "redistributable": true, "dynamic": true },
    { "id": "tbench-4-0", "axis": "implement",
      "url": "https://www.tbench.ai/leaderboard",
      "licence": "unset", "redistributable": false, "dynamic": false,
      "note": "official TB 4.0 board and the only effort-separated source found; blocked on its results licence" }
  ],

  "models": {
    "<model string as in phase_agent.model>": {
      "*":        { "weight": 65, "date": "2026-09-28", "sources": ["epoch-eci"] },
      "implement":{ "weight": 95, "date": "2026-09-28", "sources": ["epoch-eci", "tbench-4-0"] }
    },
    "*": { "*": { "weight": 50, "date": null, "sources": [] } }   // any model string not listed
  }
}
```

Resolution for a candidate `(agent, model)` in phase `p`: `models[model][p]`, else
`models[model]["*"]`, else `models["*"][p]`, else `models["*"]["*"]`, else **50**. A missing or
unparseable `weights` key resolves every candidate to 50, which makes MOD-36's selector reduce to
priority order, so an upgraded database behaves like `FirstCandidate` for slot 0 until weights are
set. Model strings are matched exactly (aliases such as `opus` are their own keys).

**The date is load-bearing.** A resolved entry whose `date` is absent or unparseable yields **0**,
not its `weight` — 0 already means "never given an extra fan-out slot" (§5.1), so an undated weight
degrades to "not offered" rather than to a stale number. Only the top-level fallback of 50 is
undated, and it exists solely for the upgrade path above. `sources` records provenance; an entry
naming a source with `redistributable: false` is refused at write time, not at read time.

Weights resolved at `StartRun` are frozen into the run's graph snapshot as a new
`SnapshotCandidate.weight: Option<u8>` with `#[serde(default)]` (absent reads as 50), so crash
recovery re-selects against the same values and the run records what drove the choice
(`R-ORCH-11`). `GraphSnapshot::V` stays 1 (`run.rs:440-445`, `:470-471`).

### 5.5 Apportionment rule for MOD-36

Given `eligible` (the walk's output, in priority order), the snapshot weights and `fanout_index`:

1. **Slot 0 is `eligible[0]`**, the first eligible candidate in priority order. Confirmed by the
   maintainer 2026-09-28 (§7.2). This keeps
   `R-AGT-8` literally true for the primary candidate and for every `fan_out = 1` phase, so weights
   never override the maintainer's explicit ordering of the lead candidate.
2. The other eligible candidates with weight > 0 are sorted by, in order: weight descending; an
   agent not yet used in slot 0 before one already used (harness and family diversity, which the
   nesting finding in §3.1 says adds more coverage than a second top model of the same family);
   priority position ascending.
3. Slots 1..N-1 take that sorted list in order. When N-1 exceeds its length, the sequence continues
   from the top of `[eligible[0]] + sorted` again, so any repeat is rival sampling on the
   highest-ranked candidates.

The rule is a pure function of its inputs, so it is crash-stable and can be pinned by table tests.
It uses no randomness, no quota utilization and no clock.

### 5.6 Refresh: a named procedure over a registry of sources

The maintainer asked for weights that are fetched and updated rather than hardcoded. That is
correct, and it is bounded by two facts the 2026-09-28 pass established: **only one source is both
directly fetchable and licensed for what htui would do with it**, and the axis that most needs
refreshing is the one that source does not cover.

**What can be fetched today.** Epoch's benchmark hub is plain HTTP, CC BY 4.0, and ships an
`eci_scores.csv` covering 268 models. It is the analysis axis. Everything else fails on licence or
on endpoint: Artificial Analysis's API is internal-use only and bars products that give
model-selection guidance, which a weight map is; Vals and SEAL publish no API; Arena has no
official results API; Harbor's tbench.ai is the official Terminal-Bench 4.0 board and the only
effort-separated source found, but its results sit behind an undocumented tRPC endpoint with no
licence grant. A probe of all of these, with the licence each carries, is in
`docs/ANA-21-recheck-2026-09-28.md` §1 and §9.

**So the shape is a registry, not a fetcher.** `weights.sources` (§5.4) lists each source with its
axis, URL, licence, and two booleans: `dynamic` (may htui fetch it?) and `redistributable` (may its
numbers be written into a seed?). Today exactly one entry is `dynamic: true`. Enabling the next one
is a data edit to that array plus a parser, never a change to the resolution rule or to the schema,
which is what keeps the infrastructure extensible instead of hardcoded to Epoch. An entry with
`redistributable: false` is refused at write time, so a licence change cannot leak into a seed by
inattention.

**The axes refresh differently, and the asymmetry is deliberate.**

- *Analysis axis* — fetched. htui pulls the Epoch export, applies the procedure below, and writes
  dated entries with Epoch cited. Nothing hardcoded survives a refresh on this axis.
- *Implement axis* — maintained, and dated. No dynamically-usable source exists, so these values
  are the maintainer's, entered through MOD-23's agents editor once it exists and by SQL until
  then, each carrying the source and date it came from. **An entry whose `date` is missing or
  unparseable resolves to 0** (§5.4), so a coding weight cannot go stale silently: it stops being
  offered for a slot rather than being quietly used. Combined with the tier convention, this is the
  §8 alias-drift mitigation applied to the values rather than to the model names.

**The procedure for deriving a weight from a source**, unchanged in substance from the draft:

1. Take independent runs only (§3.2). Use a vendor figure only where no independent run exists, and
   drop it one tier.
2. Place each model in the tier of the nearest frontier leader whose figure is within the
   benchmark's own interval; start a new tier only below that interval.
3. Treat each effort level as its own entry **only where a source separates effort levels**. Where
   none does, a sub-`high` string takes its family's `-high` value verbatim rather than being
   guessed one tier down — the rule as the draft wrote it had no anchor and was replaced for that
   reason (§7.7).
4. Record the date on the entry and name the sources it used; cite Epoch (CC BY 4.0) wherever its
   data contributed.

**Learned weights are deferred, not rejected** — but the maintainer asked for the follow-up to be
tracked rather than remembered, so it is opened as its own item rather than left to the volume
trigger. The data it needs is already recorded (§2.5); the trigger is once MOD-36 and MOD-11 are
both done and a project holds on the order of 100 judged cross-model groups, at which point a
Bayesian Bradley-Terry fit with a Plackett-Luce top-1 likelihood per verdict takes the §5.7 tier as
its prior and a same-family-as-judge covariate, shown read-only beside the manual weights before
anything is applied automatically. The fit is deterministic arithmetic over rows, so it is allowed
under `R-ID-6`.

**Finding better sources is its own item.** The single most valuable thing missing is an
effort-separated, licensed, machine-readable coding benchmark; the second is any dynamically
permitted Terminal-Bench 4.0 mirror. Both are research questions with their own licence
investigation, so they are opened as a separate ANA item rather than settled here.

### 5.7 Initial values

Settled by the maintainer on 2026-09-28 against the §3.3 re-read; every row carries the source and
date its value came from, because §5.6 makes an undated weight resolve to 0. The coding axis is
Terminal-Bench 4.0 (two independent runs) and the analysis axis is Epoch ECI with the Arena boards
as the tie-break, per §7.2.

`claude` and `claude-cli` (the same map; the model strings are Claude Code's aliases, which resolve
to the current model of each line, so the map is re-checked when an alias moves):

| Model string | `"*"` | `implement`, `fix`, `reproduce` | Basis, re-read 2026-09-28 |
|---|---|---|---|
| `opus` (Opus 5.5 today) | 95 | 95 | TB 4.0 59.6 AA / 61.62 Vals; AA II v4.3.2 57.6; Arena Code 1827 ± 18, Text 1509 ± 12. Not in Epoch ECI. Best-evidenced row in the table |
| `haiku` (Haiku 4.5) | 25 | 25 | Epoch ECI 142.42, roughly 14 points below the 156-157 frontier band. Absent from AA and from both Arena top thirties |
| `*` (including `default` **and `sonnet`**) | 65 | 65 | The conservative catch-all. `sonnet` is **deliberately not a key** (§7.4): it has no independent analysis-kind figure, so it resolves here rather than carrying a number no source can check |

`agy`:

| Model string | `"*"` | `implement`, `fix`, `reproduce` | Basis, re-read 2026-09-28 |
|---|---|---|---|
| `gemini-3.7-flash-high` | 80 | 25 | **Leads 3.8 on the analysis axis** (§7.2): ECI 157.72 vs 157.13, Arena Code 1593 ± 8 vs 1580 ± 8, Arena Text 1488 ± 5 vs 1492 ± 5. **Trails it on the coding axis**: Vals TB 4.0 6.06 vs 13.13 |
| `gemini-3.8-flash-high` | 65 | 45 | TB 4.0 19.7 AA / 13.13 Vals, AA II v4.3.2 40.9; ECI 157.13, Arena Code 1580 ± 8, Text 1492 ± 5 |
| `gemini-3.6-flash-high` | 45 | 25 | ECI 154.36, Arena Text 1482 ± 5; Vals TB 4.0 4.54, the lowest of the three Flash versions on coding |
| `gemini-3.1-pro-low` | 45 | 25 | SEAL 46.10 ± 3.60, Arena Text 1487 ± 3 (n = 119,196), ECI 154.92 - all for the **un-suffixed** model. **Dropped one tier from 65/45 for the effort gap** (§7.8, §5.6 step 3) |
| `gemini-3.8-flash-medium`, `gemini-3.7-flash-medium`, `gemini-3.8-flash-low`, `gemini-3.7-flash-low`, `gemini-3.6-flash-medium`, `gemini-3.6-flash-low` | *their family's `-high` value* | *same* | **Flattened** (§7.7). No source scores any effort level below `-high`, so the ladder is removed rather than guessed: each takes `gemini-3.{8,7,6}-flash-high` verbatim |
| `*` (including `gemini-pro-agent`, whose underlying model is undocumented) | 45 | 45 | Unknown model |

Read together: on a coding phase whose candidates are `claude/opus`, then `agy/gemini-3.8-flash-high`,
then `claude/sonnet`, fan-out 3 gives slot 0 `claude/opus`, slot 1 `claude/sonnet` (65, via the `*`
row), slot 2 `agy/gemini-3.8-flash-high` (45). On a `research` phase with the same candidates slot 1
goes to `claude/sonnet` at 65 and slot 2 to `agy/gemini-3.8-flash-high`, which now ties it;
`gemini-3.7-flash-high` at 80 would take slot 1 ahead of both, and wins the agent-diversity
tie-break over `claude/sonnet` regardless. This matches the maintainer's example ordering (Opus
above Flash) without committing to its numbers.

**The `-medium` and `-low` rows now carry no distinct weight at all.** Six of the eleven seeded
`agy` strings resolve to their family's `-high` value, which is the honest encoding of "no source
measures this" and costs the fan-out the diversity an assumed effort ladder would have bought.

## 6. What MOD-36 inherits

1. The weight shape and resolution of §5.4, the snapshot field, and the apportionment rule of §5.5,
   as the selector behind the unchanged `AgentSelector` seam, built in the run worker's `Kit` from
   the snapshot.
2. Seeds: `weights` added to `crates/htui-core/seeds/agent_{claude,claude_cli,agy}.json` with §5.7's
   values, plus a data-only pg migration that sets `settings.weights` on the existing seeded rows by
   name only where the key is absent. MOD-38 has reserved `0005`, so this is `0006` unless MOD-36
   lands first; coordinate the number at implementation time.
3. **A candidate pool.** Seeded graphs have no `phase_agent` rows, so MOD-36 must give the
   maintainer a way to list several `(agent, model)` candidates for a phase (seeded rows, or the
   graph editor MOD-14 already owns). Weights do not create candidates; `phase_agent` stays the only
   source (`R-ORCH-1`).
4. **Judge hardening**, which MOD-36's HANDOFF line already names as its model-identity-bias point:
   strip agent identity from candidate material before the judge sees it (commit trailers such as
   `Co-Authored-By`, agent names in documents), and when the project judge's agent is also one of the
   group's candidates' agents, write an `item_note` saying so, since that pairing biases the verdict
   toward its own family by more than the quality gap MOD-36 is trying to exploit.
5. Mixed-family budgeting stays as it is: every prompt uses `TokenEstimator::DEFAULT`, which is
   already the tightest (§2.6). Revisit only if an item starts choosing the estimator per model.
6. Each candidate already draws on its own agent's quota through the walk; no change.
7. **Done 2026-09-28** — §5.7's basis figures were re-read from Epoch's CSV export, the AA and
   Vals Terminal-Bench 4.0 pages, the AA Intelligence Index v4.3.2, the SEAL board and the Arena
   mirror; the record is `docs/ANA-21-recheck-2026-09-28.md`. No figure crossed a tier. Four draft
   figures did not reproduce and five seeded rows lost their basis (§3.3, §5.7). The maintainer
   settled all of them on 2026-09-28 (§7); the values in §5.7 are the settled ones and are what
   MOD-36 seeds.
8. MOD-23's editor surfaces `settings.weights` alongside the model list and default model.

## 7. Decisions taken (maintainer, 2026-09-28)

Every call this document had open is now made. Items 1-3 are the draft's; items 4-8 came out of
the primary-source re-read, where the sources did not determine a value. The refresh design
(§4.5, §5.6) was settled in the same session.

1. **`R-AGT-8` clarification — APPLIED.** The added sentence stands as proposed: "When the phase
   fans out, the remaining candidate slots are apportioned across the other eligible candidates by
   their per-model weight (ANA-21)." It is correct as written because decision 2 kept slot 0 on
   priority order. `docs/REQUIREMENTS.md` carries the amendment.
2. **Slot 0 — first eligible candidate, not the highest weight.** The draft's choice. `R-AGT-8`
   and `R-ORCH-1` stay literally true for the primary candidate and for every `fan_out = 1` phase;
   weights only ever affect rival slots.
3. **Learned weights — SPAWNED, not deferred.** The volume trigger is unchanged (§5.6) but the
   follow-up is an item, so the analysis is not lost when the trigger is reached.
4. **`sonnet` — treated as unscored, not seeded as a key.** Removed from the `claude` map; it
   resolves through the `"*"` row at 65 on both axes. This is the only option the public record
   supports, and the one §5.6's own rule reaches when the honest input is "no figure".
5. **`gemini-3.8-flash-high` coding tier — kept at 45.** Two independent Terminal-Bench 4.0 runs
   (19.7, 13.13) and AA index 40.9 now agree it is bottom-tier, but TB 4.0's own interval is wide
   near that score, and §5.6 step 2 does not license a new tier below 45 on this evidence.
6. **Gemini 3.7 vs 3.8 Flash — split by task kind.** 3.7 takes the analysis axis (ECI 157.72, Arena
   Code 1593, Arena Text 1488 all ahead of 3.8) and 3.8 takes the coding axis (Vals 13.13 vs
   6.06). Seeded as `gemini-3.7-flash-high` 80/25 and `gemini-3.8-flash-high` 65/45. The
   `default_model` on the `agy` seed is left as it is; changing it is a registry decision this item
   does not make.
7. **Unmeasured effort rows — flattened, not guessed.** Every `gemini-3.{8,7,6}-flash-{medium,low}`
   string takes its family's `-high` value verbatim on both axes. The one-tier-per-step assumption
   is withdrawn: no source measures effort below `-high`, so there was nothing to step down from.
8. **`gemini-3.1-pro-low` — one tier dropped for the effort gap.** 65/45 becomes 45/25 under §5.6
   step 3, because SEAL's 46.10 describes the un-suffixed model and the seeded string is `-low`.
   Arena Text 1487 and ECI 154.92 describe the same un-suffixed model, so all three supporting
   figures carry the same mismatch and none of them is discarded.
9. **Refresh — a source registry, not a hardcoded fetcher.** Epoch (CC BY 4.0) is fetched for the
   analysis axis and is the only `dynamic: true` entry today. The implement axis has no
   dynamically-permissible source, so it is maintained and **dated**, and an entry with no
   parseable date resolves to 0 rather than to a stale number. The registry is data, so adding a
   source is not a schema change (§5.4, §5.6).
10. **Finding better sources — SPAWNED.** An effort-separated, licensed, machine-readable coding
    benchmark is the known gap; the licence investigation is its own research question.

## 8. Risks

- **Alias drift.** `opus`/`sonnet` move to new models without any row changing, so a weight can go
  stale silently. The mitigation is now load-bearing rather than advisory: an entry whose `date` is
  missing or unparseable resolves to 0 (§5.4), so a stale coding weight stops being offered for a
  slot instead of being used.
- **Coding-axis staleness, bounded but not solved.** Epoch refreshes the analysis axis on demand;
  the implement axis has no dynamically-permissible source and is the maintainer's to maintain. The
  date rule makes the failure mode visible rather than silent, but the values are still manual.
  This is the gap the spawned sources item exists to close.
- **Source coverage moves faster than the verdict.** Opus 5.5 is absent from Epoch's ECI, Sonnet 5
  and Haiku 4.5 are absent from AA's index and from both Arena boards, SEAL has not published an
  Opus 5.x row, and the SWE-ECI file the draft leaned on is gone from the Epoch hub. The
  independent-measurement set is narrower than the frontier, and it will keep shifting.
- **A licence can change under a stored value.** The registry records `redistributable` and refuses
  a non-redistributable source at write time, but a source's terms can change after a number is
  already in a seed. A refresh that re-checks the registry would catch it; a hand-edited coding
  entry would not.
- **Harness gap.** Public numbers come from other scaffolds (§3.1); htui's own outcomes are the only
  fix, which is the learned follow-up.
- **Judge bias** once models mix; §6 item 4.

## 9. Sources

Tree: file:line references inline in §2.

**Access record.** Every source marked *re-read* below was fetched first-hand on **2026-09-28**,
with no page blocked. What reproduced, what did not, and what could not be checked is recorded in
`docs/ANA-21-recheck-2026-09-28.md`. Sources the re-read did not touch are marked *not re-read*.

Benchmarks and resolution:
- Liu et al., *Coding Agents Have Converged*, https://arxiv.org/abs/2609.17394 — *re-read*, abstract
  read in full; every figure in §3.1 reproduced exactly
- Terminal-Bench 4.0 as run by Artificial Analysis, https://artificialanalysis.ai/evaluations/terminalbench-4-0
  — *re-read*, 34 scored models parsed from the page payload
- Terminal-Bench 4.0 as run by Vals, https://www.vals.ai/benchmarks/terminal-bench-4 — *re-read*,
  board updated 2026-09-22; 34 rows, avg@3, Terminus 2
- Artificial Analysis Intelligence Index **v4.3.2**,
  https://artificialanalysis.ai/evaluations/artificial-analysis-intelligence-index — *re-read*,
  32 scored models
- Epoch Capabilities Index, https://epoch.ai/data/benchmark_data.zip — *re-read*, 268 scored models
  in `epoch_capabilities_index/eci_scores.csv`; **no SWE-ECI file in the 2026-09 export**, and
  `terminalbench_external.csv` is a 2.x file. CC BY 4.0. Also https://epoch.ai/eci,
  https://github.com/epoch-research/eci-public — *not re-read*
- SWE-bench Pro (Scale SEAL), https://labs.scale.com/leaderboard/swe_bench_pro_public — *re-read*,
  25 rows; newest Anthropic row Opus 4.6, newest Google row Gemini 3.1 Pro
- Arena mirror, https://raw.githubusercontent.com/oolong-tea-2026/arena-ai-leaderboards/main/data/2026-09-28/code.json
  and `.../text.json` — *re-read*; leaderboards last updated 2026-09-25, 30 models each
- SWE-bench results, https://github.com/SWE-bench/experiments; Vals SWE-bench archive,
  https://www.vals.ai/benchmarks/swebench — *not re-read*
- SWE-Bench Pro Verified, https://arxiv.org/abs/2609.08149 — *not re-read*
- Terminal-Bench 4.0 background, https://www.tbench.ai/news/terminal-bench-4-0,
  https://github.com/harbor-framework/terminal-bench; version churn
  https://flowtivity.ai/blog/terminal-bench-4-score-crash/ — *not re-read*; Vals' methodology
  section independently confirms the 66-task set, seven categories and the 8-hour limit
- SWE-rebench, https://swe-rebench.com/; DeepSWE, https://deepswe.datacurve.ai/blog/deepswe-v1-1 —
  *not re-read*
- Judge evaluation: https://github.com/ScalerLab/JudgeBench, https://arxiv.org/abs/2506.01937,
  https://arxiv.org/abs/2606.19544 — *not re-read*

Pricing and quotas:
- Anthropic pricing, https://platform.claude.com/docs/en/about-claude/pricing — *re-read* in full
- Gemini pricing, https://ai.google.dev/gemini-api/docs/pricing — *re-read*
- models.dev, https://github.com/sst/models.dev; LiteLLM cost map,
  https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json;
  OpenRouter models API, https://openrouter.ai/docs/guides/overview/models — *not re-read*
- `openai.com` pricing for GPT-6 Sol — **not fetched in either pass**; §3.4's 2/10 stays
  second-hand

Routing and learning from preferences:
- LiteLLM routing, https://docs.litellm.ai/docs/routing; Portkey load balancing,
  https://portkey.ai/docs/product/ai-gateway/load-balancing; OpenRouter provider selection,
  https://openrouter.ai/docs/guides/routing/provider-selection
- RouteLLM, https://arxiv.org/abs/2406.18665; Martian, https://work.withmartian.com/
- LMArena's move to Bradley-Terry, https://www.lmsys.org/blog/2023-12-07-leaderboard/,
  https://github.com/lmarena/arena-rank; Bayesian BT, https://arxiv.org/abs/1011.1761;
  The Leaderboard Illusion, https://arxiv.org/abs/2504.20879
- Judge bias: Zheng et al., https://arxiv.org/abs/2306.05685; Panickssery et al., NeurIPS 2024;
  *Who Judges Matters*, https://arxiv.org/abs/2609.17857; Wang et al.,
  https://arxiv.org/abs/2305.17926; CodeJudgeBench, https://arxiv.org/abs/2507.10535;
  PoLL, https://arxiv.org/abs/2404.18796
- Model diversity in selection: DEI, https://arxiv.org/abs/2408.07060; LLM-as-a-Verifier,
  https://arxiv.org/abs/2607.05391; Self-MoA, https://arxiv.org/abs/2502.00674
