# ANA-24 - A licensed, effort-separated coding benchmark source for the weight map (concluded, 2026-10-01)

Opened 2026-09-28 by ANA-21 (`docs/decisions/ana/ana-21.md`, decision 10). ANA-21 made the weight
map's refresh a registry of sources (`docs/ANA-21.md` §5.4, §5.6), each entry flagged `dynamic`
(htui may fetch it) and `redistributable` (its numbers may ship in a seed). Only Epoch's ECI
passed, for the analysis axis. The implement axis had no permissible source, and no permissible
source separated reasoning effort. Addresses `R-AGT-8`, `R-ORCH-7`. Analysis: `docs/ANA-24.md`.

**Method.** Seven parallel first-hand probes, one per source family, quoting licence text and
recording every URL hit. Then one refuting verifier per source claimed `dynamic` or
`redistributable`: fifteen claims, five refuted outright and one narrowed (`docs/ANA-24.md` §3.6).

**Verdict (`docs/ANA-24.md` §5).**

1. **Two implement-axis sources may be both fetched and shipped**, both CC BY 4.0 and both current:
   - **Epoch MirrorCode** (`mirrorcode.csv` in the hub zip htui already registers): Epoch's own
     agentic coding run. Pass rates, 9 models, one effort level each (Opus 5.5 max 0.774, Fable
     5.1 high 0.733, GPT-6 Astra high 0.467). It is the primary coding signal.
   - **Arena's `lmarena-ai/leaderboard-dataset`** on Hugging Face: `webdev` (Code Arena) and
     `agent` (Agent Arena) subsets. These are preference ratings, not pass rates. Effort is in the
     row names, at several levels per family. They place models MirrorCode lacks, and they retire
     ANA-21 §5.6's effort flattening for the families they separate.
2. **tbench.ai stays blocked.** The board is readable without a key: a Supabase function, wrapped
   by Harbor's documented CLI, so not tRPC. But the Harbor Hub terms make results the submitter's
   Customer Content, for internal use. Recorded only; no outreach.
3. **No permissible Terminal-Bench 4.0 mirror exists.** llm-stats' site grant is contradicted by
   its own API terms; BenchLM is CC BY-NC; the others are unlicensed or derived from Artificial
   Analysis.
4. **Epoch's CC BY covers Epoch's own runs and fitted index only.** `*_external.csv` files keep
   their upstream licences. That blocks the zip's CursorBench, DeepSWE, FrontierSWE, FrontierCode
   and Terminal-Bench 2.0 copies, `processed_data_for_eci.csv`, and any rebuilt SWE-ECI.
5. **Registry**: `epoch-mirrorcode`, `arena-webdev` and `arena-agent` are added; `epoch-eci` is
   narrowed to `eci_scores.csv`; `tbench-4-0` gets its real terms. Six licensed but stale sources
   are recorded and not registered: OpenHands Index, Aider, Multi-SWE-bench, Open Agent
   Leaderboard, Epoch SWE-bench Verified and TB 2.0 on HF.

**Corrections to ANA-21** (`docs/ANA-24.md` §7):
- Arena has had an official CC BY dataset since 2026-04-02.
- LiveBench has had an Agentic Coding category since 2025-05-30 (still unlicensed).
- SEAL has a current V2 board (still unlicensed).
- swebench.com's leaderboard data is CC BY-NC.
- No SWE-ECI file ever existed in the Epoch hub.
- ECI now scores Opus 5.5, at 167.35. No tier crosses.

**Maintainer decisions (2026-10-01, `docs/ANA-24.md` §8).** Option B, with MirrorCode primary.
No new item: the entries and their parsers land in MOD-36, which owns `weights.sources`, and a
dated note on MOD-36 points at `docs/ANA-24.md` §5–§6. The tbench.ai findings are recorded only.

**MOD-36 inherits these fetch conditions (`docs/ANA-24.md` §6):**
- Fetch Arena from Hugging Face only, never arena.ai, whose terms ban scraping.
- Check `cardData.license == cc-by-4.0` on every fetch, because the publishing pipeline dropped the
  licence once.
- Pin the subset and the `latest` split.
- Key Epoch parsing on `benchmark_metadata.csv`, and never read `*_external.csv`.
- Credit Epoch AI and Arena under CC BY.

Commits: on branch `hr/ANA-24`. The analysis doc and this close-out land in one commit.
