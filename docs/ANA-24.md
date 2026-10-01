# ANA-24 - A licensed, effort-separated coding benchmark source for the weight map

> **Scope note:** "ANA-21's refresh is a source registry, and exactly one entry in it can be
> fetched today: Epoch (CC BY 4.0), which covers the **analysis** axis only. The implement axis has
> no dynamically-permissible source at all [...] Research the gap": find a machine-readable,
> redistribution-licensed source of agentic coding results (the prize being Harbor's tbench.ai,
> the only board found with `reasoning_effort` per row); establish whether any permissible
> Terminal-Bench 4.0 mirror exists (Artificial Analysis out of scope); re-probe the sources ANA-21
> recorded as blocked; and deliver a verdict naming which sources are `dynamic: true` and which
> `redistributable: true`, so ANA-21's `weights.sources` registry (§5.4) can be extended as a data
> edit. (`HANDOFF.md`, ANA-24; opened 2026-09-28 by ANA-21, `docs/decisions/ana/ana-21.md`.)
>
> **Requirements addressed:** `R-AGT-8`, `R-ORCH-7`. Touched at their seams: `R-AGT-5` (a source
> is registry data, never code keyed on a name).
>
> **Status (2026-10-01): concluded.** Every source was probed first-hand on 2026-10-01, and every
> source claimed `dynamic` or `redistributable` was then put to an independent agent instructed to
> refute the claim (§3.6). **The implement axis gains two sources that may be both fetched and
> shipped: Epoch's MirrorCode run and Arena's official Hugging Face leaderboard dataset**, both CC
> BY 4.0 and both carrying the current frontier. MirrorCode is the primary coding signal (task pass
> rate, 9 models, one effort level each); Arena's Code Arena and Agent Arena subsets fill the models
> MirrorCode lacks and are the one permissible current source that separates effort levels.
> **tbench.ai stays blocked**: its board is now readable without a key, but its results are the
> submitter's property under the Harbor Hub terms and carry no grant. No permissible Terminal-Bench
> 4.0 mirror exists. Several of ANA-21's §3.2 conclusions did not survive the re-probe (§7). No new
> item is spawned: the registry entries and their parsers land in MOD-36, which owns
> `weights.sources` and has not started (§8).

---

## 1. Context and problem statement

ANA-21 made per-model weights a coarse ordinal tier keyed on phase name, stored in
`agent.settings.weights`, and refreshed through a **registry of sources** rather than a hardcoded
fetcher (`docs/ANA-21.md` §5.4, §5.6). Each registry entry carries two booleans:

- `dynamic` — htui may fetch the source automatically at run time. This needs a stable
  machine-readable endpoint **and** terms that permit automated fetching for use in a product that
  gives model-selection guidance, which a weight map is.
- `redistributable` — the source's numbers may be written into a seed that ships with htui. This
  needs an explicit licence or terms grant covering the **results** (not merely the harness code).

On 2026-09-28 exactly one source passed both tests: Epoch's benchmark hub, used for the analysis
axis. The implement axis — the one that matters most for `implement`, `fix` and `reproduce` phases
— had no permissible source, so its weights are the maintainer's, entered by hand and dated, with
an undated entry resolving to 0. And because no permissible source separated reasoning effort,
ANA-21 §5.6 step 3 flattened every sub-`high` string onto its family's `-high` value.

This analysis closes as much of that gap as the licences allow.

## 2. Method

Seven probes ran in parallel, each assigned one source family and required to record every URL it
hit with its HTTP status, to quote licence text rather than report it from memory, and to
distinguish a code licence from a results licence:

| Probe | Scope |
|---|---|
| tbench | tbench.ai, Harbor Hub, the harbor-framework GitHub org and HF datasets (question 1) |
| tb-mirrors | any Terminal-Bench 4.0 copy elsewhere: GitHub, HF, Kaggle, llm-stats, BenchLM, Snorkel, Epoch, OpenRouter, models.dev, LiteLLM, vendor cards (question 2) |
| vals-seal | Vals and Scale SEAL, including any results datasets (question 3) |
| arena-livebench | Arena / LMArena (site, HF datasets, arena-rank, the third-party mirror) and LiveBench (question 3) |
| swe | SWE-bench/experiments, swebench.com's leaderboard data, SWE-rebench and its Nebius datasets (question 3) |
| epoch | the Epoch hub zip file by file, `epoch-research/eci-public`, the site's SWE-domain ECI (question 3) |
| discovery | anything else: Aider, DeepSWE, FrontierSWE, CursorBench, METR, HAL, OpenHands Index, Multi-SWE-bench, SWE-PolyBench, Open Agent Leaderboard, BigCodeBench, LiveCodeBench, SWE-Lancer, Copilot Arena, Kaggle Game Arena |

Every source a probe marked `dynamic` or `redistributable` was then handed to a separate verifier
told to **refute** each true flag — terms-of-service carve-outs, research-only or non-commercial
clauses, scraping bans, code-only licences, undocumented endpoints, narrower upstream licences —
and to default a flag to false where the grant is not explicit and checkable. Fifteen claims were
verified; §3.6 records the outcome. 24 agents, all completed. The GitHub REST API rate-limited
(403) during parts of the sweep; licences were then read from `raw.githubusercontent.com` files and
the Hugging Face API instead, and no verdict below rests on an unread licence.

## 3. Findings

### 3.1 tbench.ai (question 1)

**The fetch problem is solved; the licence problem is not.**

*Endpoint.* ANA-21 looked for a tRPC route and found only 404s; the transport was misidentified.
`https://www.tbench.ai/leaderboard` now 308-redirects to the homepage, which hosts the board, and
the site's bundles call a Supabase edge function:

```
POST https://ofhuhcpkvzjlejydnvyd.supabase.co/functions/v1/leaderboard-read
     {"package":"terminal-bench/terminal-bench","name":"4-0-0"}
```

It needs no key, sends `access-control-allow-origin: *`, and returned 200 JSON with 27 rows
(`metadata.reasoning_effort`, `metrics.accuracy`, CI95, pass@k, tokens, cost). The raw function is
undocumented and tied to a Supabase project ref. Harbor's own Apache-2.0 CLI wraps it as
`harbor hub leaderboard show terminal-bench/terminal-bench/4-0-0 --json`, and the Harbor docs say
"Public leaderboards can be read without signing in"
(`docs.harborframework.com/core-concepts/harbor-hub/leaderboards`). So a documented read path
exists.

*Effort separation.* Confirmed. `reasoning_effort` is a required field of the board's metadata
schema, with values `low`, `medium`, `high`, `xhigh`, `max`. Examples: Codex / GPT-6 Astra max
58.18, xhigh 57.88, high 57.88, medium 54.24, low 50.61; Claude Code / Fable 5.1 max 57.88, high
54.55, low 43.33; Claude Code / Opus 5 xhigh 53.94 down to low 34.85. The board has no Claude Opus
5.5 row; it was last updated 2026-09-21 (Grok 4.7).

*Licence.* tbench.ai has no terms page (`/terms`, `/privacy`, `/robots.txt` all 404), and neither
the board nor the TB 4.0 announcement mentions a licence. The board lives on Harbor Hub, whose
Terms of Service (HarborCo Inc., last updated 2026-07-10, `hub.harborframework.com/terms`) are the
only terms that reach it:

- "'Customer Content' means all data ... together with the outputs generated by Customer's
  Executions, including agent trajectories, transcripts, logs, scores, rewards, and evaluation
  results ('Results')."
- "2.2 Ownership. As between the parties, Customer owns all Customer Content."
- Harbor's own licence to results is "solely as necessary to (a) provide, maintain, and secure the
  Service", and access is granted "for Customer's internal business purposes".

Results are owned by whoever submitted them, and nothing grants a third party the right to reuse
them in a product. **Verdict: `dynamic: false`, `redistributable: false`.**

Two adjacent first-party copies were also found:

- `harbor-framework/terminal-bench`, `leaderboard/submissions/*.json` — TB 4.0 submission records
  pinned to the dataset by sha256, each with `reasoning_effort`, in a repo whose root LICENSE is
  Apache-2.0. But nothing says Apache-2.0 covers the results, and the copy is partial: 13 files,
  newest 2026-09-02, mostly one `max` run per model, no GPT-6 Astra, no Grok 4.7, none of the
  multi-effort rows. Blocked: an arguable grant over an incomplete copy.
- `harborframework/terminal-bench-2-leaderboard` on Hugging Face — explicitly `license: apache-2.0`
  on the card since its first commit (2025-12-26), before any third-party submission was merged; a
  verifier could not refute either flag. But it is Terminal-Bench **2.0**, frozen since 2026-05-15,
  newest rows Opus 4.7 / GPT-5.5, and not comparable with 4.0 (`docs/ANA-21.md` §3.2). Permissible
  and useless for the current frontier. It is the precedent worth citing if Harbor is ever asked
  for a 4.0 data grant.

### 3.2 Terminal-Bench 4.0 mirrors (question 2)

**None is permissible.** Every candidate fails on licence, version, or provenance:

| Mirror | TB 4.0? | Effort | Licence over the data | Verdict |
|---|---|---|---|---|
| llm-stats.com (ZeroEval) | labelled 4.0; about half the rows are vendor self-reports | one score per model, effort in free-text notes | site ToS (2026-09-30) grants attribution-only reuse, **but** its API docs say "Regardless of plan, do not ... Redistribute the dataset, or substantial extracts, without a Commercial agreement" and ban scraping the site to bypass the API; the governing API-terms page 404s | refuted on verify — not redistributable, not dynamic |
| BenchLM.ai | yes (snapshot 2026-09-30) | page yes, `models.json` no | CC BY-NC 4.0 | non-commercial |
| Snorkel AI | yes | yes | personal non-commercial use only | blocked |
| CodingFleet | yes | yes | all rights reserved | blocked |
| BenchmarkList | vendor self-reports | partial | none found | blocked |
| LLMLearner | mostly Artificial Analysis-derived | yes | none | out of scope |
| mekos2772/bench-benchmark-sync | via Artificial Analysis | — | MIT covers code only | out of scope |
| leoncuhk/awesome-llm-bench | 2.0 only | no | defers to BenchLM (NC) | blocked |
| final-bench/ALL-Bench-Leaderboard | 2.0 | no | MIT tag | stale |
| Epoch hub `terminalbench_external.csv` | **2.0 only**, rebuilt 2026-10-01 | no | external data keeps its upstream licence (§3.5) | refuted on verify |
| Vendor system cards | vendor-run | one per model | consumer terms ban harvesting | blocked |
| OpenRouter, models.dev, LiteLLM | carry no TB 4.0 numbers | — | — | n/a |
| Kaggle, HF community datasets | no TB 4.0 results | — | — | n/a |

OpenRouter's models API now carries a `benchmarks` object, but it is Artificial Analysis-sourced and
therefore out of scope.

### 3.3 Re-probe of ANA-21's blocked sources (question 3)

| Source | 2026-09-28 (ANA-21) | 2026-10-01 | Verdict |
|---|---|---|---|
| **Vals** | HTML only, not granted | Board updated 2026-09-29: Opus 5.5 now 65.15 (was 61.62), plus Sonnet 5.5, GPT-6.1 Sol, Gemini 4 Argon. Rows carry `reasoning_effort` / `compute_effort`, one config per model. Data reachable only as JSON in Astro page props. No terms page; footer "Copyright © 2026 Vals AI. All rights reserved." | blocked |
| **Scale SEAL** | HTML only, stale | A **SWE-Bench Pro V2** board (launched 2026-09-22) is current: Opus 5, Fable 5.1, GPT-6 Astra, Gemini 3.8 Flash, effort and harness in the row name ("Opus 5 (Claude Code) xhigh"); no Opus 5.5; Full variant saturated (top three within 1.7 points). v1 stays stale. Scale Website Terms: use "solely with supported browsers ... for your own internal purposes", Scale owns the "information, data"; robots.txt disallows `/api/`. The MIT repo and HF task dataset cover tasks, not results. | blocked |
| **Arena** | no official results API; GitHub mirror is a third-party scrape | **Wrong on 2026-09-28.** Arena publishes `lmarena-ai/leaderboard-dataset` on Hugging Face, CC BY 4.0 (§3.4). The arena.ai site itself stays forbidden: its Terms of Use (2026-02-23) bar programmatic access, scraping and mirroring, and robots.txt disallows `/api/`. The `oolong-tea-2026` mirror now has an MIT LICENSE, which covers its code and cannot grant rights over data scraped from arena.ai. `arena-rank` is Apache-2.0 code with no data. | HF dataset **usable**; site and mirror blocked |
| **SWE-bench/experiments** | no licence | Still no LICENSE (`LICENSE`, `LICENSE.md`, `LICENSE.txt`, `COPYING` all 404). New: swebench.com's leaderboard data (`SWE-bench/swe-bench.github.io`, `data/leaderboards.json`) **is** licensed — CC BY-NC 4.0 — and the rendered site says "All rights reserved". Stale: newest Verified row 2026-02-26; Verified closed to non-academic submitters since 2025-11-18. | blocked (non-commercial / unlicensed) |
| **SWE-rebench** | no API, not granted | Unchanged: scores only in the Next.js payload, no terms page, footer "© 2026 SWE-rebench Team". Effort appears in row names ("Opus 5 [high]", "gpt-5.5-...-xhigh"), fixed ReAct scaffold, but newest task window ends 2026-07-01 and no Opus 5.5, GPT-6 Astra, Fable 5.1 or Gemini 3.8 Flash. The Nebius HF datasets are CC BY 4.0 but hold **task instances**, not scores. | blocked; HF datasets not a weight source |
| **LiveBench** | NOASSERTION; general capability, not agentic coding | **Second half wrong.** LiveBench has had an Agentic Coding category since 2025-05-30 (Mini-SWE-Agent; "agentic v2" from the 2026-06-25 release), with effort-separated current rows (`claude-opus-5-5-max-effort` 71.72 vs `-xhigh-effort` 65.35; `gpt-6.1-sol-max` vs `-xhigh`). But the CSV path (`livebench.ai/table_<release>.csv`) is undocumented and release-keyed, and the scores carry **no licence**: the site's CC BY-SA footer is copied from the Nerfies template, where it covers the site's code; the deploy repo `LiveBench/new-livebench` has no LICENSE; the DATASHEET's Apache-2.0 covers the 960 questions, not results. Two open GitHub issues ask for a results licence, unanswered. | refuted on verify — blocked |
| **epoch-research/eci-public** | not re-read | MIT-licensed fitting code, ships no data, downloads the hub zip at run time; last commit 2026-09-07. | not a source |

### 3.4 Sources that pass both bars with current frontier rows

**Epoch MirrorCode** (`mirrorcode.csv` in `https://epoch.ai/data/benchmark_data.zip`). Epoch's own
long-horizon agentic coding benchmark, co-developed with METR, run by Epoch. Being an internal run
(no `_external` suffix; `benchmark_metadata.csv` maps it to `source_file mirrorcode.csv`,
`score_column "Best score (across scorers)"`), it sits under Epoch's CC BY 4.0 grant, which the
MirrorCode page repeats: "Epoch AI's work is free to use, distribute, and reproduce provided the
source and authors are credited under the Creative Commons Attribution license". Nine rows, one
effort level each:

| Row | Score |
|---|---|
| claude-opus-5-5_max | 0.774 |
| claude-fable-5-1_high | 0.733 |
| claude-fable-5_high | 0.639 |
| gpt-6-astra_high | 0.467 |
| claude-opus-4-7_high | 0.311 |
| gpt-5.6-sol_high | 0.200 |
| gpt-5.4-2026-03-05_high | 0.156 |
| gpt-5.5_high | 0.100 |
| gemini-3.1-pro-preview_high | 0.089 |

No Gemini 3.8 / 3.7 Flash, no Sonnet, no Haiku. It is in the zip htui already registers, so it is a
second parser over the same download, not a new fetch. **Verified: both flags hold.**

**Arena leaderboard dataset** (`https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset`).
Arena's official daily snapshot as Parquet, created 2026-04-02, CC BY 4.0 in the card metadata. The
grant is deliberate and confirmed in writing: in HF discussion #2 an Arena staff member
(`cthorrez-arena`, a verified `lmarena-ai` org member) answered a request from an open-source
**model-recommendation tool** — essentially htui's use — with "This dataset is under cc-by-4.0
license ... Excited to hear about your interest in using this data for your project!" Two subsets
bear on coding:

- `webdev` (Code Arena) — Bradley-Terry ratings from human preference over generated web apps.
  580 latest rows published 2026-09-30: claude-opus-5.5-max 1817.8, gpt-6-astra-max 1789.1,
  gpt-6.1-sol-max 1758.7, claude-fable-5.1-max 1750.7. Effort is in the name, and the same model
  appears at several levels (`claude-opus-5-max` vs `claude-opus-5-high`; `gemini-3.8-flash-high`).
- `agent` (Agent Arena) — IPS scores from real agent sessions since 2026-06-04, 46 latest rows:
  1 Claude Fable 5.1 (Max) 0.1455, 2 Claude Opus 5.5 (High) 0.1378, 3 GPT 6 Astra (Max) 0.1218,
  19 Gemini 3.8 Flash (High) 0.0296. Multi-domain: only about 17.5% of sessions write code.

These are preference ratings, not pass rates, and the two subsets name models differently
(`claude-opus-5.5-max` vs `Claude Opus 5.5 (High)`). **Verified (twice, independently): both flags
hold**, under the conditions in §6.

### 3.5 Epoch's hub: what CC BY covers, and what it does not

The single most consequential detail of the sweep: Epoch's grant is narrower than the zip README
suggests. The README carries only "Epoch AI's data is free to use, distribute, and reproduce
provided the source and authors are credited under the Creative Commons Attribution license", but
`https://epoch.ai/benchmarks/use-this-data` continues: "Benchmark questions and answers are the
property of their respective creators. This hub also includes data sourced from external projects,
which retains its original licensing. Users are responsible for complying with the license terms of
the specific data they use". Consequences:

- **Epoch internal runs** (`mirrorcode.csv`, `swe_bench_verified.csv`) — CC BY 4.0. Usable.
- **`*_external.csv`** — keep their upstream licence. This blocks the zip's copies of the best
  effort-separated coding boards found: CursorBench 4.0 (full sweep, e.g. `claude-sonnet-5-5_low`
  to `_max`; Cursor's terms forbid extraction), DeepSWE v1.1 (full sweep, e.g. `gpt-6-astra_low` to
  `_max`; no results licence), FrontierSWE v2 (no licence), FrontierCode 1.1 (Cognition's site
  terms: personal use only), and Terminal-Bench 2.0.
- **ECI** — `eci_scores.csv`, `edi_scores.csv`, `eci_bootstraps.json` are Epoch's fitted output and
  CC BY 4.0. `processed_data_for_eci.csv` (also served as `/data/eci_benchmarks.csv`) is mixed: its
  `source` column names 75 sources, including tbench.ai (28 rows) and Vals ProofBench (61 rows).
  Shipping that file, or per-benchmark numbers taken from it, would redistribute third-party data.
  The existing `epoch-eci` entry is sound only as long as it reads `eci_scores.csv`.
- **The site's "Software engineering ECI"** is computed in the browser and never shipped as a file.
  It can be rebuilt from the zip, but most of its inputs for the frontier models are external
  (Opus 5.5's three SWE inputs are MirrorCode, FrontierCode and FrontierSWE), so a rebuilt SWE-ECI
  has no clear licence. Refuted on verify.

Epoch's `swe_bench_verified.csv` is an internal run and permissible, but stale (newest row
2026-06-16, no Opus 5.x or GPT-6) and saturated (top 0.83). Airtable, which Epoch documents as a
second route, is not public ("Airtable doesn't allow public API access"); the zip is the only
dynamic endpoint.

### 3.6 Other sources, and the verification record

Licensed but without current frontier rows — permissible, not useful today:

| Source | Licence | Newest frontier row |
|---|---|---|
| OpenHands Index (HF `OpenHands/openhands-index`) | Apache-2.0, written into the card by the publish script; the GitHub repo has no LICENSE and its README claims MIT — use the HF copy only | claude-fable-5 (2026-06-11); frozen since 2026-06-30; no effort field |
| Aider polyglot (`aider/website/_data/polyglot_leaderboard.yml`) | Apache-2.0 repo-wide, contributions under a CLA | 2025-10-03; effort field present |
| Multi-SWE-bench (`multi-swe-bench/experiments`) | Apache-2.0, README states it covers the results | GPT-5.2 (2026-01) |
| Open Agent Leaderboard (HF `open-agent-leaderboard/results`) | CDLA-Permissive-2.0 (licence text must travel with shared data) | Opus 4.5 / GPT-5.2; last update 2026-05-18 |
| Epoch SWE-bench Verified | CC BY 4.0 | 2026-06-16; saturated |
| Terminal-Bench 2.0 HF leaderboard | Apache-2.0 | Opus 4.7; frozen 2026-05-15 |

No results licence (blocked): DeepSWE (best data found: a full effort sweep as a clean static JSON,
but no licence anywhere), FrontierSWE, CursorBench (terms forbid scraping), METR time horizons
(README points at a LICENSE file that does not exist), HAL (no licence; harness archived),
BigCodeBench, LiveCodeBench, SWE-Lancer, Copilot Arena, SWE-PolyBench results.

Verification record — every source a probe marked `dynamic` or `redistributable`:

| Claim | Probe said | After verify |
|---|---|---|
| Epoch MirrorCode | dyn ✓ red ✓ | **holds** |
| Arena HF leaderboard-dataset (two verifiers) | dyn ✓ red ✓ | **holds** — fetch from huggingface.co only, check the licence on every fetch |
| Epoch ECI | dyn ✓ red ✓ | dyn holds; red holds **only for Epoch-fitted files**, not `processed_data_for_eci.csv` |
| Epoch hub implement files | dyn ✓ red ✓ | holds for internal runs only; Airtable route refuted |
| Epoch SWE-bench Verified | dyn ✓ red ✓ | holds (stale) |
| Epoch SWE-ECI (derived) | dyn ✓ red ✓ | **refuted** — external inputs |
| Epoch `terminalbench_external.csv` | dyn ✓ red ✓ | **refuted** — external, 2.0 |
| llm-stats TB 4.0 | red ✓ | **refuted** — API terms bar redistribution |
| LiveBench leaderboard CSV | red ✓ | **refuted** — template footer, no data grant |
| LiveBench HF and repo | red ✓ | **refuted** — Apache-2.0 covers questions, not scores |
| TB 2.0 HF leaderboard | dyn ✓ red ✓ | holds (stale, wrong version) |
| Nebius SWE-rebench HF | dyn ✓ red ✓ | holds for tasks; **not a weight source** (no score columns) |
| OpenHands Index | dyn ✓ red ✓ | holds for the HF copy only (stale) |
| Aider polyglot | dyn ✓ red ✓ | holds (stale) |
| Multi-SWE-bench | dyn ✓ red ✓ | holds (stale) |
| Open Agent Leaderboard | dyn ✓ red ✓ | holds (stale) |

## 4. Options

**A. MirrorCode only.** Pass-rate data only on the implement axis. Clean, but places just the
frontier models MirrorCode ran, one effort level each, and nothing for Gemini Flash or Sonnet.

**B. MirrorCode primary, Arena as the second implement source.** MirrorCode places the models it
covers; Arena's `webdev` and `agent` subsets place models MirrorCode lacks and separate effort
levels within a family. The cost is mixing a pass rate with preference ratings, which the tier
procedure absorbs: ANA-21's tiers are ordinal and coarse precisely so that a model's tier is
"near which frontier leader, within the benchmark's own interval", which works per source.

**C. Register everything permissible.** Adds six stale-but-licensed sources (§3.6). They carry no
current frontier row, so they would add parsers and attribution duties without moving a single
seeded tier.

**D. Keep waiting on tbench.** The only full effort sweep on a fixed, current, agentic task set.
Blocked on a licence only Harbor (or each submitter) can grant.

## 5. Verdict

**Option B, maintainer-confirmed 2026-10-01.** The registry gains two implement-axis entries, the
existing `tbench-4-0` and `epoch-eci` entries are corrected, and nothing else is registered.

```jsonc
// agent.settings.weights.sources — ANA-24 (2026-10-01); supersedes ANA-21 §5.4's list
[
  { "id": "epoch-eci", "axis": "analysis",
    "url": "https://epoch.ai/data/benchmark_data.zip",
    "file": "epoch_capabilities_index/eci_scores.csv",
    "licence": "CC BY 4.0", "redistributable": true, "dynamic": true,
    "note": "Epoch-fitted files only; processed_data_for_eci.csv mixes external data" },

  { "id": "epoch-mirrorcode", "axis": "implement", "primary": true,
    "url": "https://epoch.ai/data/benchmark_data.zip",
    "file": "mirrorcode.csv",
    "licence": "CC BY 4.0", "redistributable": true, "dynamic": true,
    "note": "Epoch internal run; pass rate; one effort level per model" },

  { "id": "arena-webdev", "axis": "implement",
    "url": "https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset",
    "subset": "webdev", "split": "latest",
    "licence": "CC BY 4.0", "redistributable": true, "dynamic": true,
    "note": "Code Arena; Bradley-Terry preference ratings; effort in model name" },

  { "id": "arena-agent", "axis": "implement",
    "url": "https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset",
    "subset": "agent", "split": "latest",
    "licence": "CC BY 4.0", "redistributable": true, "dynamic": true,
    "note": "Agent Arena; IPS scores; multi-domain, consulted only where webdev has no row" },

  { "id": "tbench-4-0", "axis": "implement",
    "url": "https://hub.harborframework.com/datasets/terminal-bench/terminal-bench/latest?tab=leaderboard&leaderboard=4-0-0",
    "licence": "Harbor Hub ToS 2026-07-10: results are the submitter's Customer Content; internal use only",
    "redistributable": false, "dynamic": false,
    "note": "readable via `harbor hub leaderboard show ... --json`; blocked on a results licence" }
]
```

`file`, `subset`, `split` and `primary` are new optional keys: data, read by the parser for that
source, never by the resolution rule. The shape of §5.4 is otherwise unchanged and the resolution
rule does not change at all.

**How the implement axis uses them** — ANA-21 §5.6's procedure, applied per source:

1. A model MirrorCode scores is placed by MirrorCode.
2. A model MirrorCode lacks is placed by `arena-webdev`; only where `webdev` has no row does
   `arena-agent` place it.
3. **Effort.** Where an Arena subset carries the same family at more than one effort level, each
   level is its own entry, placed by that subset. This retires §5.6 step 3's flattening for those
   families. Where no permissible source separates effort, the flattening rule stands unchanged.
4. Disagreement between MirrorCode and Arena on a model MirrorCode scores is not averaged:
   MirrorCode wins and the entry's `sources` names only `epoch-mirrorcode`.
5. A model no registered source covers stays maintained by hand and dated, exactly as ANA-21 left
   it.

What this does **not** buy: a full effort sweep on a fixed agentic pass-rate benchmark. Only
tbench.ai, DeepSWE and CursorBench publish one, and none carries a grant.

## 6. What MOD-36 inherits

The conditions that make the two new sources safe to fetch. Each is a verifier finding, not a
preference:

- **Arena: fetch from Hugging Face only** (`huggingface.co/.../resolve/main/...` or
  `datasets-server.huggingface.co`). arena.ai embeds the same leaderboard JSON in its pages, and
  that copy is under the arena.ai Terms of Use, which ban automated access and scraping.
- **Arena: check the licence on every fetch.** The grant is a single `license: cc-by-4.0` line in
  the card's YAML, and Arena's publishing pipeline has dropped it once before (commits
  `965ff13304`, `ab89e14ce0`, 2026-04-08; restored `d06a798e45`, 2026-04-22). Read
  `cardData.license` from `https://huggingface.co/api/datasets/lmarena-ai/leaderboard-dataset` and
  refuse the data unless it is `cc-by-4.0`. A seed records the dataset commit SHA it was built from.
- **Arena: pin subset and split; resolve shards.** The dataset's default config has already moved
  (from `text_style_control` to `agent`). Use `latest`, not `full`: Arena announced on 2026-09-30
  that the Agent Arena Steerability method changed and backfilled history will be rewritten. Shard
  file names are numbered; resolve them through the tree or datasets-server API.
- **Arena: the per-row `license` column is the model's licence, not the data's.**
- **Epoch: key on `benchmark_metadata.csv`, not column positions.** The zip is regenerated in place
  with no schema promise; read `source_file` and `score_column` from metadata and fail closed when
  either is missing. Never read a `*_external.csv` file, nor `processed_data_for_eci.csv`.
- **Attribution.** All three registered dynamic sources are CC BY 4.0: credit Epoch AI and Arena,
  link the licence, and say that the scores were transformed into weights. A shipped seed carries
  the same credit.
- **Name mapping** stays MOD-36's problem as ANA-21 left it, and grows: MirrorCode writes
  `claude-opus-5-5_max`, Code Arena `claude-opus-5.5-max`, Agent Arena `Claude Opus 5.5 (High)`.

## 7. Corrections to ANA-21

ANA-21 §3.2 and §5.6 are historical records and stay as written; these are the facts that moved.

| ANA-21 said | 2026-10-01 |
|---|---|
| tbench.ai results sit behind an undocumented tRPC endpoint | A public Supabase function, wrapped by Harbor's documented CLI. The blocker is the licence alone. |
| tbench.ai is the only effort-separated board found | Also Vals (one config per model), SEAL V2, SWE-rebench, LiveBench Agentic Coding, DeepSWE, CursorBench — and, permissibly, Arena's HF dataset |
| Arena: no official results API | Official CC BY 4.0 HF dataset since 2026-04-02 |
| LiveBench: general capability only, not agentic coding | Agentic Coding category since 2025-05-30, effort-separated. Still unlicensed. |
| SEAL is stale against the frontier | v1 is; SWE-Bench Pro V2 (2026-09-22) is current. Still unlicensed. |
| SWE-bench/experiments: no licence | Unchanged; swebench.com's leaderboard data is CC BY-NC 4.0 |
| Epoch covers the analysis axis only | Its own MirrorCode run covers the implement axis too |
| ECI has no Opus 5.5; Astra 166.6, Fable 5.1 165.0, Gemini 3.8 Flash 157.13 | 270 models: Opus 5.5 167.35 (first), Astra 166.51, Sonnet 5.5 165.2, Fable 5.1 164.82, Gemini 3.8 Flash 156.93. No tier crosses. |
| 2026-09-28 drop of SWE-ECI from the Epoch hub | There was never a SWE-ECI file (zip copies of 08-20, 09-17, 09-29, 10-01 checked); it is computed in the browser |
| `epoch-eci` entry: the zip, CC BY 4.0 | CC BY covers Epoch-fitted files only (§3.5) |
| Vals TB 4.0: Opus 5.5 61.62 | 65.15 (board updated 2026-09-29) |

## 8. Decisions taken (maintainer, 2026-10-01)

1. **Sources.** Register Epoch MirrorCode and Arena's HF leaderboard dataset for the implement
   axis, MirrorCode primary (option B). The six licensed-but-stale sources are recorded here and
   not registered.
2. **Where the work lands.** No new item. MOD-36 owns `weights.sources` and has not started; a
   dated note on MOD-36 points its seed registry and per-source parsers at §5 and §6.
3. **tbench.ai.** Recorded only: the endpoint and licence findings stand in §3.1 and the entry
   stays `dynamic: false`, `redistributable: false`. No licence-request text is drafted.

## 9. Phasing

Nothing to phase here: ANA-24 writes no code. MOD-36 picks up §5 and §6 when it runs. A licence
change at tbench.ai (or a grant on DeepSWE, CursorBench, LiveBench or SWE-rebench) is again a data
edit to the registry plus a parser — the shape ANA-21 chose for exactly this case.

## 10. Risks

- **Preference is not pass rate.** Arena ratings measure what voters prefer, over web apps (Code
  Arena) or mixed agent sessions (Agent Arena). A model can rate well and resolve fewer tasks. The
  mitigations are structural: MirrorCode wins where both speak, Arena only places models into
  coarse tiers, and the maintainer still owns any entry by editing it.
- **Licence drift.** Both new grants can be withdrawn or silently dropped. The per-fetch licence
  check (§6) turns an Arena drop into a refused fetch instead of a quiet use. Epoch's grant has no
  machine-readable equivalent; a re-read belongs in any refresh that changes a tier.
- **MirrorCode is narrow.** Nine models, frontier only, one effort level each; a new frontier model
  may take weeks to appear.
- **Agent Arena history rewrites.** The 2026-09-30 method change means `full` values will move;
  `latest` only is read.
- **Endpoint drift.** Neither source versions its schema. Both parsers fail closed (§6), and a
  failed fetch leaves the previous dated entries in place, which age out to 0 per ANA-21 §5.4.

## 11. Sources

All fetched first-hand on **2026-10-01** unless marked otherwise.

tbench.ai / Harbor:
- https://www.tbench.ai/ (board; `/leaderboard` 308s here), https://www.tbench.ai/news/terminal-bench-4-0
- `POST https://ofhuhcpkvzjlejydnvyd.supabase.co/functions/v1/leaderboard-read` (200, 27 rows)
- https://hub.harborframework.com/terms (Harbor Hub ToS, 2026-07-10)
- https://docs.harborframework.com/core-concepts/harbor-hub/leaderboards
- https://github.com/harbor-framework/terminal-bench (`leaderboard/submissions/`, Apache-2.0 root)
- https://huggingface.co/datasets/harborframework/terminal-bench-2-leaderboard (Apache-2.0, TB 2.0)

Terminal-Bench 4.0 mirrors:
- https://llm-stats.com/terms; https://docs.llm-stats.com/api-reference/errors.md;
  https://docs.llm-stats.com/api-reference/attribution.md; https://llm-stats.com/legal/api-terms (404)
- https://benchlm.ai/terms; https://snorkel.ai/terms/;
  https://codingfleet.com/blog/terminal-bench-4-leaderboard-2026/;
  https://benchmarklist.com/api/v1/results/terminal_bench_4.json; https://llmlearner.com/terms

Re-probed:
- Vals: https://www.vals.ai/benchmarks/terminal-bench-4, https://www.vals.ai/methodology
- Scale: https://labs.scale.com/leaderboard/swe_bench_pro_public_v2,
  https://labs.scale.com/leaderboard/swe_bench_pro_public, https://scale.com/legal/terms
- Arena: https://huggingface.co/datasets/lmarena-ai/leaderboard-dataset (card, API, discussions
  #2, #3), https://help.arena.ai/articles/5629909088-terms-of-use,
  https://arena.ai/company/leaderboard-changelog, https://github.com/lmarena/arena-rank,
  https://github.com/oolong-tea-2026/arena-ai-leaderboards
- SWE-bench: https://github.com/SWE-bench/experiments,
  https://github.com/SWE-bench/swe-bench.github.io (`LICENSE`, `data/leaderboards.json`)
- SWE-rebench: https://swe-rebench.com/, https://huggingface.co/datasets/nebius/SWE-rebench-leaderboard
- LiveBench: https://livebench.ai/ (`table_2026_06_25.csv`),
  https://github.com/LiveBench/LiveBench (`docs/DATASHEET.md`), https://github.com/LiveBench/new-livebench
- Epoch: https://epoch.ai/data/benchmark_data.zip (rebuilt 2026-10-01),
  https://epoch.ai/benchmarks/use-this-data, https://epoch.ai/eci, https://epoch.ai/MirrorCode,
  https://github.com/epoch-research/eci-public, https://github.com/epoch-research/epochai-python

Discovery:
- https://huggingface.co/datasets/OpenHands/openhands-index,
  https://github.com/OpenHands/openhands-index-results
- https://github.com/Aider-AI/aider (`LICENSE.txt`, `polyglot_leaderboard.yml`)
- https://github.com/multi-swe-bench/experiments
- https://huggingface.co/datasets/open-agent-leaderboard/results
- https://deepswe.datacurve.ai/ (`/artifacts/v1.1/leaderboard-live.json`),
  https://github.com/datacurve-ai/deep-swe
- https://github.com/Proximal-Labs/frontier-swe-v2; https://cursor.com/terms-of-service
- https://github.com/METR/eval-analysis-public; https://hal.cs.princeton.edu/;
  https://github.com/princeton-pli/hal-harness
- BigCodeBench, LiveCodeBench, SWE-Lancer, Copilot Arena, SWE-PolyBench repositories (licence files
  read via `raw.githubusercontent.com`)

Out of scope throughout: Artificial Analysis (internal-use API; bars model-selection guidance).
