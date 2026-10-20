# ANA-38 - Project and workspace memory (concluded, 2026-10-20)

Full analysis: `docs/ANA-38.md` (surface as read, prior art, options, risks, sources, what was not verified).
Origin: the 2026-10-15 requirements review, `docs/requirements-review-2026-10-15.md`. Addresses `R-ENT-13`
(reopened), `R-ID-6`, `R-STO-8`, `R-PRM-1`, `R-PRM-3`; touches `R-ORCH-14`, `R-ORCH-1`, `R-ENT-12`,
`R-ENT-15`, `R-ID-7` and `R-NF-2` at their seams.

## Verdict

The maintainer answered all ten questions of `docs/ANA-38.md` §8 "all as recommended" on 2026-10-19, then
changed the direction with three decisions, and answered the five residual questions "all as recommended" on
2026-10-20.

- **Memory comes back as verbatim entries, not ANA-1's atomic facts.** An entry has a subject, prose, a kind
  (`fact`, `decision`, `gotcha` and the new `lesson`: recurring errors, optimizations already done, things
  that improve usage over time), an owner scope (project, workspace, and a new **global** scope for lessons
  and gotchas that apply in any project), a state (`active`, `suspect`, `superseded`, `retired`; no draft
  state) and a freshness stamp. Not brought back: importance scores 1-10, the fact-to-fact link graph,
  recursive-CTE injection. Summaries stay; entries sit beside them.
- **Memory is completely automatic.** A model (the configurable light model of `R-ORCH-1`, spending the
  user's plan) writes entries at run end, fed also by a deterministic lesson detector; entries become active
  with no person's acceptance. This **supersedes** the recommended answers to questions 1, 2 and 9 ("a model
  only drafts, a person accepts, MOD-135's draft store"), and it **amends `R-ID-6`** (a "must") with an
  explicit memory exception. The edit is owed and maintainer-gated; `docs/ANA-38.md` §7.3 has the proposed
  text. The person-accepts variant stays in the analysis as Option E, the fallback.
- **Safeguards replace the gate, all automatic.** A deterministic write path around the model (schema
  validation, a per-run write cap, scrub per `R-ID-7` before persist, provenance stamped by the platform,
  evidence that must cite something observed in the run); dedup and supersede at write time (subject key plus
  full-text; raw-cosine when Qdrant is present); computed freshness (a version stamp on cited rows, an
  optional commit-and-path pin for git projects, checked by a sweep and never inside prompt assembly), a
  suspect entry held back from prompts, an automatic re-validation turn and auto-retire; deterministic usage
  counters for demotion; human visibility without gating (list, edit, retire, pin-as-trusted, "why is this
  here", per-level off switches in the Settings cascade; default on).
- **Global scope** is a third hidden scope row on the pattern of ANA-35's workspace scope row. Promotion is
  automatic through a generalization check and a deterministic validator that rejects project, workspace,
  path, host, URL and secret tokens; a failing entry keeps its scope; an agent never targets global
  directly. Global lessons carry the agent or CLI version they were learned on and go suspect when it
  changes.
- **Prompt reach is Postgres-only:** a new internal section filled in `phase_spec`, active non-suspect
  entries by scope in a fixed order, a hard token cap, a run-start snapshot, quoted data labelled with
  source and trust, trimmed after Excerpts and before Upstream, recorded in `trim_record`. Global has its own
  small cap, ranked below project and workspace.
- **Qdrant is an add-on**, not a dependency: a memory point type, agent-pull search through
  `search_concepts`, and raw-cosine dedup at write time, riding ANA-35's `htui_concepts_v4` rebuild. Memory
  works with Qdrant absent; ANA-38 gives ANA-37 something to measure.
- **Residual answers (2026-10-20), starting defaults, tuned at MOD-169:** 5 writes per run; about 1,500
  prompt tokens for project memory, 500 for workspace, 300 for global; a suspect entry auto-retires after 14
  days (kept for audit); a global scope waits for MOD-169's first read on MOD-163..167; a per-run
  extraction budget, with extraction skipped for runs that saw no error, retry or measured win; a pinned
  entry skips auto-retire and usage demotion but is still held back when suspect, and any member allowed to
  write to the project or workspace may edit, retire or pin it (`R-ENT-10` compare-and-set).
- **Requirement edits owed** (maintainer-gated, `docs/REQUIREMENTS.md` not edited at close-out):
  `R-ENT-13` final text, the `R-ID-6` exception, `R-PRM-1`/`R-PRM-3` (memory in the input list, a rank, and
  the global clause), `R-STO-8`, `R-ENT-12` (a global owner), CONCEPTS lines (Memory, Lesson, Global scope,
  "Bounded context"), a TUI line and a Settings line, a note in ANA-2 invariant 3 / MOD-98 (the extraction
  turn is a run-end step before close-out and the close-out formatter stays model-free), and ANA-34 §7.2.

## Spawned items

| Filed | From | Subject | Blocked on |
|---|---|---|---|
| MOD-163 | P1 | Entry store and prompt section (must) | none for project scope |
| MOD-164 | P2 | Automatic capture at run end (must) | MOD-163; the `R-ID-6` amendment; MOD-147 for workspace entries; MOD-136 for the coordinator seed |
| MOD-165 | P3 | Lesson detector over run records | MOD-164; its first task verifies the run-record detail |
| MOD-166 | P4 | Freshness sweep, pins and re-validation | MOD-163; MOD-164 for the re-validation turn |
| MOD-167 | P5 | Qdrant point type and raw-cosine dedup | MOD-163, MOD-164, MOD-147; MOD-112 |
| MOD-168 | P6 | Global scope and automatic promotion | MOD-147, MOD-164, MOD-166 |
| MOD-169 | P7 | Measurement on real htui steps | MOD-163, MOD-164 |

`R-ENT-13` priority: "must" for MOD-163 and MOD-164; "later" for MOD-165..168 until MOD-169 reports.

Effects on open items: ANA-34's coordinator seed gains a concrete reader of active entries (MOD-136 supplies
the turn, MOD-163 the entries); **MOD-135's draft store no longer blocks memory** (it stays for follow-up
drafts and `item_create`); ANA-37 gains MOD-167 and MOD-169 as the things to measure against; MOD-147 gates
workspace and global scope; MOD-126's light model gains a third use (workflow choice, draft rewrite,
memory writes).

## Reconciled with ANA-37 (2026-10-20)

ANA-38 was written without ANA-37's verdict (see Not verified). The maintainer reconciled the two on
2026-10-20; where they conflict, this section governs `docs/ANA-38.md` §4.6, §6, §7 and §8 Q6.

- **One Qdrant, two kinds of collection.** One Qdrant for the whole database, run like Postgres.
  Project and workspace entries sit in their workspace's collection; global entries in one global
  collection, never copied into a workspace's. Not `htui_concepts_v4` or a single collection.
- **Indexing.** ANA-37's lease-holding indexer (MOD-159), one lease per collection, indexes memory on
  its 60 s cycle; MOD-167 supplies the point type. No run indexes its own entries.
- **No supersede from a score.** Automatic supersede happens only on the same (scope, subject) key.
  The raw-cosine check runs in the indexer when it indexes a new entry and records near duplicates as
  candidates for the visibility list and MOD-169 (`docs/ANA-37.md` §5.4: no score separated duplicates).
  The full-text fallback is dropped. MOD-169 may justify supersede-from-score later, with a requirement
  amendment.
- **No run waits on Qdrant** (`R-NF-2`): the extraction turn makes no Qdrant query.
- **Agents find memory only through knowledge search** (`search_concepts`, global memory included);
  no word-search fallback for memory. The prompt section stays Postgres-only and fixed-order; ranking it
  by Qdrant relevance is a later option measured by MOD-169.
- **Requirements amended:** `R-NF-2`, `R-STO-1`, `R-STO-8`, `R-TUI-8`, `R-ENT-13` (where memory is
  indexed, no supersede from a score), `R-MCP-1`, `R-MCP-2`. Of this item's owed edits, `R-STO-8`'s memory
  point type is done and its "raw-cosine write-time dedup" clause is dropped; `R-ENT-13`'s final text and
  the `R-ID-6` exception stay owed and maintainer-gated.
- **Items amended:** MOD-163, MOD-164, MOD-167, MOD-168, MOD-169.

## Owed edits decided (2026-10-20)

The maintainer then answered the owed, maintainer-gated edits of the Verdict:

- **`R-ID-6` exception: allowed**, applied as §7.3 proposed it, with the light model's third use.
  MOD-164 is unblocked on it.
- **`R-ENT-13`: "must" as a whole**, automatic with its verifications (write-path checks, computed
  freshness, re-validation, promotion gate, visibility). This replaces "must for P1 and P2, later for
  P3..P6 until P7": MOD-163..169 are all "must"; MOD-168 is still built after MOD-169's first read, and
  MOD-169 measures and tunes rather than deciding whether memory stays.
- **The rest, applied:** `R-PRM-1`/`R-PRM-3` (memory in the prompt, after excerpts and before upstream
  in the cut order, global cut first; global memory the one input from outside the workspace),
  `R-ENT-12` (a global owner), `R-TUI-8` (memory switches and global entries in Settings), `R-TUI-14` (a
  memory sub-tab per workspace and project), CONCEPTS ("Bounded context", "Memory"), and dated notes in
  ANA-2 invariant 3, ANA-5 §4.4, ANA-34 §7.2 and MOD-98.
- **Ranking prompt memory by Qdrant relevance:** deferred to MOD-169.
- **Collection versions:** none while htui is in development; a payload change drops and reindexes
  (MOD-144, MOD-147, MOD-159).

## Commits

`8833d8bd` (draft), `59632c88` (draft v2), and the conclusion commit that adds this file and files
MOD-163..169.

## Not verified

See `docs/ANA-38.md` §9.2: what error and retry detail run records store today (MOD-165's first task); the
light model's cost per run; the validator's recall for project-specific tokens; whether a vendor CLI's own
memory loads in an htui-started session; the benefit figures from the web survey, which are vendor-reported
or abstract-only and not measured on htui's workload; ANA-37's verdict and its filed items, which live on the
`hr/ANA-37` branch and were not readable here (`/host/htui` is at the ANA-35 merge); Qdrant request
timeouts and the sync interval.
