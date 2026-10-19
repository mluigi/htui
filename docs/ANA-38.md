# ANA-38 - Project and workspace memory

> **Scope note:** Settle what `R-ENT-13`'s reopened "memory" is for htui: facts, decisions and gotchas
> that outlive one item and are read by the coordinator (`R-ORCH-14`) and by agents. Four points are
> named by the requirement itself: who writes an entry, how it stays fresh, how it meets `R-ID-6`, and how
> it meets semantic search (`R-STO-8`). A fifth is left open by it: whether memory is its own entity or a
> kind of document, and how it reaches a prompt (`R-PRM-1`, `R-PRM-3`). Governed by
> `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md` and
> `docs/requirements-review-2026-10-15.md` (the item's origin). Best read beside ANA-34 (the coordinator
> that reads memory), ANA-35 (owner documents, a candidate home) and ANA-37 (is Qdrant required), which is
> not on this branch. This is draft v2: the maintainer answered section 8 on 2026-10-19 and changed the
> direction (fully automatic memory, a `lesson` kind, a global scope). The answers are recorded in
> section 8, with the residual questions at its end; nothing else here is a decision until they are answered.
>
> **Requirements addressed:** `R-ENT-13` (reopened, pending ANA-38), `R-ID-6`, `R-STO-8`, `R-PRM-1`,
> `R-PRM-3`. Touched at their seams: `R-ORCH-14` (coordinator seed), `R-ENT-12` (owner documents),
> `R-ENT-15` (version-stamped citations, the freshness precedent), `R-MCP-1`/`R-MCP-2` (agent writes),
> `R-TUI-9` (close-out), `R-ID-3`/`R-ID-4`/`R-ID-5` (Postgres is the store, no files in repos, htui
> inlines prompts), `R-ID-7` (scrub before persist), `R-STO-3` (offline cache mirror), `R-NF-2` (Qdrant
> fails open), `R-ORCH-1` (the configurable light model that would write entries).
>
> **Status (2026-10-19: draft v2, maintainer answered section 8; residual questions in section 8).** No
> implementation code is written here and no requirement text is edited. The maintainer answered the ten
> questions of section 8 "all as recommended" and then amended them with three further decisions that
> override parts of questions 1, 2 and 9 and adjust 4, 6 and 8. Recommended verdict (section 7), in short:
> bring memory back as **a small, fully automatic store of verbatim entries, written by a model through a
> validated deterministic pipeline, with no person-acceptance gate**, not as ANA-1's atomic facts. An entry
> has a subject, prose, a kind (`fact`, `decision`, `gotcha`, and the new `lesson`: recurring errors,
> optimizations already done, things that improve usage over time), an owner scope (project, workspace, and
> a new **global** scope for lessons and gotchas that apply in any project), a state (`active`, `suspect`,
> `superseded`, `retired`; there is no draft state) and a freshness stamp. **The writer is a model** (the
> configurable light model of `R-ORCH-1`, which spends the user's plan) at run end, fed also by a
> deterministic lesson detector; **entries become active without approval**. This amends `R-ID-6`, a
> "must": memory writes become an explicit exception to "no LLM in bookkeeping", listed as an owed,
> maintainer-gated edit. The earlier recommendation, "a model only drafts, a person accepts, MOD-135 draft
> store", is superseded and kept in section 4 as Option E, the predecessor of Option E-prime.
> **Safeguards replace the gate and are automatic, not approval gates**: a deterministic write path around
> the model (schema validation, per-run write cap, scrub before persist, platform-stamped provenance,
> evidence that cites something observed in the run), dedup and supersede at write time, computed freshness
> with an automatic re-validation turn and auto-retire, deterministic usage counters, a gated automatic
> promotion to global scope, and human visibility (list, edit, retire, pin-as-trusted, "why is this here",
> per-level off switches) without gating. **Freshness is computed by code, never a model**: a version stamp
> on whatever the entry cites (the `makes_suspect` / `reconfirm` pattern that exists today), plus, for git
> projects only, an optional commit-and-path pin checked by a sweep that stores a suspect flag. **Prompt
> reach is Postgres-only**: a new internal prompt section filled in `phase_spec`, active non-suspect
> entries only, a hard token cap, a snapshot at run start, trimmed early. **Qdrant is an add-on** that
> improves dedup recall and agent-pull search; memory works with Qdrant absent, so ANA-38 gives ANA-37
> something to measure without depending on its answer. **Not** brought back: importance scores 1-10, the
> fact-to-fact link graph, recursive-CTE injection. The costs that remain are named plainly in section 6:
> unreviewed standing instructions, memory poisoning with no human gate, a model call on every run, global
> leakage, runaway growth and self-reinforcing wrong lessons. Phasing P1..P7 is in section 7; the P's are
> proposed and not yet filed, and no IDs are minted.

---

## 1. Context and problem statement

ANA-1 (now superseded, "do not implement from this file") specified an `artifact` table of atomic repo
facts (headline, content, `importance` 1-10 with a CHECK, nullable `item_id` for global repo facts,
`run_step_id`, `commit_hash`) and an `artifact_link` table (`supports`, `refines`, `contradicts`,
`relates_to`). Facts with importance 7 or more, reached over one or two upstream hops by a recursive CTE,
were injected into prompts beside upstream summaries (`docs/ANA-1.md:64-75, 283-313, 342-404`; the
`>= 7` filter is at `:385`).

**The drop.** The 2026-09-03 requirements session (commit `f4e24c54`, which also superseded ANA-1 and
ANA-8) withdrew it. The original requirement text, recovered from that commit
(`docs/REQUIREMENTS.md`, lines 106-107 at that revision), gave the whole reason in one line: "Dropped as
over-engineered; summaries cover the need. Re-open only on evidence." The withdrawal was then applied
twice: ANA-9 listed the `artifact` tables as a contradiction of the contract and dropped them
(`docs/ANA-9.md:22-25, 985`), and ANA-11 invariant 5 "Nothing generic" cited it as the precedent for
refusing a generic node or attribute store (`docs/ANA-11.md:67-68, 99`). No transcript, decision file or
longer rationale for the withdrawal was found.

**The reopening.** On 2026-10-15 the maintainer reopened it in the requirements review
(`docs/requirements-review-2026-10-15.md:290-293`): study "project/workspace memory (facts, decisions,
gotchas) for the coordinator and agents: who writes it, how it stays fresh, R-ID-6, and integration with
Qdrant". `R-ENT-13` now reads "reopened, pending ANA-38" and makes no must/later call and no data-model
call (`docs/REQUIREMENTS.md:270-274`; header note `:53`). The reopening brought no new evidence, which the
original said it needed; it is a maintainer instruction. This document therefore has to earn the
reopening on its merits, and says where it cannot (section 6, section 9).

**What "over-engineered" most plausibly meant** (inference, not stated anywhere): the machinery around the
facts, meaning the 1-10 scores, the fact-to-fact link graph and the prompt CTE, not the idea of a durable
gotcha. ANA-1 itself named the freshness problem and answered it only with a `commit_hash` flag ("allowing
htui to detect and flag facts verified on outdated git commits"); no expiry, confirmation or writer was
designed, and the DDL never says who writes a fact (`docs/ANA-1.md:283-298, 612`; `CONCEPTS.md:85-87`).
The two gaps the reopening names, who writes and how it stays fresh, are exactly the ones ANA-1 left open.

**Why "summaries cover the need" is weaker than it sounds.** Summaries reach a prompt only through
`blocked_by`/`origin` edges within one or two hops inside the active workspace (`docs/REQUIREMENTS.md:520-523`),
so a gotcha from an unlinked item never surfaces. And the only code that writes a document of kind
`summary` is the close-out formatter: a commit table plus one line per run, produced by no step (confirmed
by a spot check of non-test code in `htui-orch` and `htui-core`; `crates/htui-orch/src/closeout.rs:1-16`).
So today's "summary" carries commits, not decisions or gotchas. Two close-out artefacts were already
recorded as having no database home: the "live coordinates" recap and the "watch items for later modules"
note (`docs/ANA-2.md:1525-1529`; `.claude/rules/workflow-docs.md:129-131`). In this repo's own workflow,
ANA-29 finds that "many working rules live only as memory notes that every agent prompt has to restate, and
the evidence shows that this fails silently" (`docs/ANA-29.md:84-86, 283-294`). That last point is about the
maintainer's development setup, not the product's runtime agents; it motivates the question but does not
measure it.

**Not the same thing as agent-session memory.** MOD-24 dropped "checkpoint agent memory, re-hydrate, resume
mid-step" on 2026-09-25 (`docs/decisions/mod/mod-24.md:4-7`), and resume is decided from store rows, never
memory (`docs/ANA-34.md:252-253`, `docs/ANA-33.md:276`). ANA-38 memory is a durable knowledge store.

Method: six readers in parallel (history; substrate in code; seed, meaning coordinator, close-out and
prompt reach; coding-harness memory prior art; memory frameworks and Qdrant; freshness and poisoning),
each returning facts with `path:line` or URL evidence, then one critic pass with spot checks of
`R-ID-6`, `R-ENT-13`, `R-PRM-1/3`, `SectionName`/`RECORD_VERSION` and the `summary` writer, all of which held.
The sections below are the synthesis. What was **not** verified is listed in section 9: Gortex does not
index `docs/*.md`, so docs were read by window and some mentions of memory may sit in files not opened;
ANA-36, ANA-37, ANA-19/20, ANA-3 and the MOD-34/50/68 write-ups were skimmed or unread; nothing was run
(no Qdrant live test, no build, no measurement on htui data); several benefit numbers are vendor-reported
or abstract-only.

**Revision (v2, 2026-10-19).** The maintainer answered section 8 and moved the design from "a model drafts,
a person accepts" to complete automation. The evidence in sections 2 and 3 and the options of section 4 are
kept as written for draft v1; the person-accepts hybrid stays as Option E and the recommendation is the new
Option E-prime. Where a passage still describes the person gate, the answers in section 8 and the text of
sections 4.6, 6 and 7 govern. New statements about the repo made in this revision are labelled unverified.

## 2. Surface as read

Line numbers marked `~` in the sweep were approximate and are given here by section or file only.

### 2.1 The requirement seam: R-ENT-13, R-ID-6, R-PRM-1/3, R-ORCH-14

- `R-ENT-13`: memory is "facts, decisions and gotchas that outlive one item, for the coordinator
  (R-ORCH-14) and for agents"; it must settle who writes it, how it stays fresh, how it meets `R-ID-6`,
  and integration with semantic search (`R-STO-8`) (`docs/REQUIREMENTS.md:270-274`). It names the
  coordinator and agents as readers, not people.
- `R-ID-6`: no LLM agent in a sync, cache, import or bookkeeping path. Allowed outside it: a display-only
  label, and "a draft a person accepts or edits before it is saved, as long as no status transition,
  link, import, cache refresh or prompt ever reads the unaccepted model text"; drafts "are written only
  once a person accepts them". Choosing which work to do next is item work, not bookkeeping
  (`docs/REQUIREMENTS.md:121-131`; `docs/requirements-review-2026-10-15.md:31-33`). A prompt is on the
  forbidden-reader list. The maintainer's decision of 2026-10-19 makes memory writes an explicit exception
  to the first sentence; the amendment is owed and maintainer-gated (section 7.3).
- `R-PRM-1` lists the prompt's inputs (phase template, item body, input documents, upstream summaries
  within one to two hops and inside the active workspace or project, box profile, bound skills, selected
  file excerpts) and says "Never raw transcripts of other items"; more context is pulled through MCP
  read tools (`docs/REQUIREMENTS.md:520-526, 566-576`; `CONCEPTS.md:82-84`). Memory is not in the list,
  so adding it amends `R-PRM-1` and CONCEPTS "Bounded context".
- `R-PRM-3`: template and skills are never cut; then item body, prior documents, upstream summaries,
  excerpts, the last cut first; the trim is recorded on the step (`docs/REQUIREMENTS.md:527-533`).
- `R-ORCH-14` (must): the coordinator "only manages"; it names no memory duty
  (`docs/REQUIREMENTS.md:483-488`). Memory enters ANA-34 only through one sentence: "ANA-38's memory, when
  it lands, is added to the seed" of a coordinator turn (`docs/ANA-34.md:559-560`), and ANA-34 "leaves a
  hook, not a design" (`docs/ANA-34.md:263-264`). A project with no coordinator agent gets the
  deterministic checklist alone (`docs/ANA-34.md:530-541`), so memory must be readable without one.
- Hard constraints on any entry: Postgres is the single store and no markdown files are produced
  (`R-ID-3`, `R-TUI-9`); htui writes no files into a managed repo (`R-ID-4`, `docs/REQUIREMENTS.md:113-115`);
  persisted text is scrubbed on the host first, failing closed (`R-ID-7`); concurrent edits use version
  compare-and-set (`R-ENT-10`); requirement text changes only by maintainer decision
  (`docs/REQUIREMENTS.md:108-112, 132, 258-260`). The R-ID-7 and R-ID-4 line numbers are computed from block
  offsets, not individually re-read.

### 2.2 The prompt seam

- `assemble(spec, scrubber)` is a pure function: no store handle, no I/O, no clock, no run or step id, so
  fan-out siblings get byte-identical prompts and the digest is reproducible on any box. Everything
  arrives pre-resolved in `PromptSpec` (`crates/htui-core/src/prompt/mod.rs:1-12, 61-77, 505`;
  `docs/ANA-5.md:128-137, 142-145`).
- The engine builds the spec in `phase_spec`, which reads the item, input documents, template,
  `upstream_summaries(item, hops, PromptScope::project_only)`, skills and budget. The engine is headless
  and holds no workspace, so the upstream walk is project-bounded today
  (`crates/htui-orch/src/engine.rs:5894-6042`, comment at the top of that function; `5783-5802, 5819-5871`).
- `SectionName` is a closed enum (Template, Persona, Item, Documents, Upstream, Box, Skills, Excerpts,
  VerifyFailure, PreviousDiff, CommandQueue, Output, plus judge/handoff-only ones); protected sections are
  Template, Persona, Box, Skills, CommandQueue, Output (`crates/htui-core/src/prompt/mod.rs:278-362`).
  Phase trim order, first to lose: Excerpts, Upstream, PreviousDiff, VerifyFailure, Documents, Item
  (`crates/htui-core/src/prompt/trim.rs:1-6, 326-370`). The trim is recorded in `run_step.trim_record`,
  `RECORD_VERSION` is 4 (`trim.rs:57-62`, spot-checked at `trim.rs:59`). Persona and Output are internal
  placeholders no template can name (`crates/htui-core/src/prompt/template.rs:72-160`).
- ANA-5 invariant 7: "No model chooses what enters a prompt"; Serena/Graphify providers are "the right
  shape and an embedding reranker the wrong one" (`docs/ANA-5.md:149-152`). ANA-5 rejected a model-written
  `{{step_summary}}` by name (`docs/ANA-5.md:1301-1304`); ANA-30 ruled out model-written summaries, model
  excerpt ranking, model duplicate proposals and automatic triage (`docs/ANA-30.md:362-367`).
- The `ExcerptProvider` seam is for file excerpts only: synchronous, no store handle, 1.5 s deadline,
  returns `ExcerptCandidate{repo, path, lines, weight, reason}`; production registers only `BuiltinRanker`
  (`crates/htui-core/src/prompt/excerpt.rs:502-571`; `crates/htui-agent/src/excerpt.rs:740-860, 979-995`;
  `crates/htui-core/src/prompt/settings.rs:46`). It is not a text-memory seam.
- Skills are the one existing person-written, scoped (global, workspace, project, item kind, phase),
  versioned, protected prompt channel; they are refused, not cut, over `max_skill_tokens`
  (`docs/REQUIREMENTS.md:537-545`; `crates/htui-core/src/prompt/trim.rs:25-30`; `docs/decisions/ana/ana-40.md`).
  A "gotchas" skill is today's zero-new-code memory. Whether its token budget and scoping suffice was
  not checked.

### 2.3 The store seam

- Existing agent-writable, prose, item-owned rows: `item_note` (per-item thread, no scope above an item,
  `created_by`, `via_step_id`), `document` (`item_id NOT NULL` today, versioned, open `kind` set,
  append-only), and `session_event` transcripts (`crates/htui-store/migrations/0001_init.sql`, `item_note`
  and `document` blocks, about lines 378-400).
- Decision-provenance patterns already in the schema: `item_revision`, and
  `requirement_revision.amended_by_item_id` ("the deciding item")
  (`crates/htui-store/migrations/0006_requirements.sql:100-127`).
- The only freshness-by-version machinery: `item_requirement` stores `requirement_version`; `makes_suspect`
  is `version > stamp`, derived at read time; `WriteStore::reconfirm` re-stamps a live citation and is
  person-gated (`crates/htui-core/src/model/requirement.rs:139-143`; `crates/htui-core/src/store/traits.rs:1805-1812`;
  `crates/htui-core/src/store/mem.rs:6823, 6856`; `crates/htui/src/ui/tabs/backlog/detail/requirements.rs:241`).
  Suspect-link detection is code, never an LLM (ANA-11 invariant 3, `docs/ANA-11.md:57-72`). It has no
  expiry, age or commit awareness. Close-out does not call it today (`docs/ANA-34.md:116-121`).
- Git facts recorded per run: `run_step_commit(before_hash NOT NULL, after_hash)` and
  `run_step_tree(base_ref, dirty)` per step per repo; `repo` has `remote_url` and `default_branch` only.
  **No branch name is stored per run or item.** Folder repos have no hashes at all
  (`crates/htui-store/migrations/0001_init.sql:185-196, 498-507`;
  `crates/htui-store/migrations/0003_orchestration.sql:88-101`; `docs/ANA-35.md`, section 2.3).
- `upstream_summaries` reads the latest `kind='summary'` document of linked items, `MAX_UPSTREAM_HOPS = 2`
  (`crates/htui-store/src/pg/read.rs:542-614`; `crates/htui-core/src/store/traits.rs:91`).
- Migration and mirror cost of any new table: Postgres migrations 0001..0018 exist (ANA-35 MOD-144..150
  will add more), forward-only; the SQLite read-only cache mirror (0001..0005) must carry items, notes,
  documents, requirements and citations for offline browsing (`R-STO-3`, `docs/REQUIREMENTS.md:324-340`).
  A new store-trait method touches PgStore, MemStore, CacheStore, Writer, WorkerStore, Backend and
  UsageSpy, plus the conformance suite and `.sqlx` entries (symbol search of `upstream_summaries`, for
  example `crates/htui-store/src/pg/read.rs:542`, `crates/htui-core/src/store/mem.rs:8139`,
  `crates/htui-agent/src/conformance.rs:674`). The implementor count for a new method is extrapolated.
- Owner-level home: ANA-35 (concluded 2026-10-18) makes a document owned by an item, a project or a
  workspace (through a hidden workspace scope row, slug `@<workspace>`), versioned, with an open kind set
  (`goals` conventional, `summary` reserved for item close-out), written by a person or an accepted
  draft, with `document_write` over MCP staying item-only (`docs/REQUIREMENTS.md:263-266`;
  `docs/ANA-35.md:684-695, 731-739, 957-961`; `docs/decisions/ana/ana-35.md`, Q12-Q14, Q16). Not built:
  MOD-146/147, open. A project may sit in several workspaces today (`workspace_project` PK is
  `(workspace_id, project_id)`), so workspace memory reaching a project step has no single owner until that
  scope row exists. A global scope (v2) would be a third row of the same pattern; ANA-35 does not design
  one (checked only against the cited section).

### 2.4 The Qdrant seam

- One collection `htui_concepts_v2`, tenant = `project_id`, dense (cosine, 384-d `Xenova/bge-small-en-v1.5`,
  local embedder) plus BM25 sparse, fused with RRF. Point types: item, document section, requirement.
  Notes and transcripts are **not** indexed, though `R-STO-8` lists them (MOD-112 part 4, open)
  (`crates/htui-store/src/vector.rs:1-10, 39-41, 83-90, 1059-1107`; `crates/htui-store/src/embed.rs:59`;
  `crates/htui-store/src/vector_sync.rs:67-131`; `docs/REQUIREMENTS.md:344-356`; `HANDOFF.md:748-757`).
  Memory is not in `R-STO-8`'s indexed list.
- Rebuild from Postgres: `Indexer::sync` is incremental and idempotent; a requirement point is rebuilt when
  `version` differs; rows no longer listed lose their points; point ids are deterministic (SHA-256-derived
  UUIDs with a domain separator, so they cannot collide with item UUIDs); a payload change takes a **new
  collection name** instead of a migration (`vector_sync.rs:1-13, 133-165`; `vector.rs:34-39, 262-292`;
  `docs/decisions/mod/mod-34.md`, `mod-50.md`, cited by section). ANA-35 plans `htui_concepts_v4`
  for item-less document points (`HANDOFF.md:468`; `docs/ANA-35.md:969-976`). Whether a v3 name is
  claimed elsewhere was not traced.
- Who syncs: only a headless worker with a Qdrant URL in its keyring syncs automatically (every
  `concepts_sync_minutes`, 15 by default, taken from a test name); a provisioned worker has no keyring and
  never indexes or offers `search_concepts` (`crates/htui/src/concepts.rs:17-19, 300-409, 1126`;
  `crates/htui/src/worker_cmd.rs:96`; `docs/htui-mcp.md`, section "Which session gets which tools").
- Fail-open is by construction: Qdrant errors are `StoreError::Backend`, never `Unreachable`; search runs
  on its own task; errors return as `ConceptsReply::Hits{outcome:Err}`; the MCP tool answers `search
  unavailable: <cause>`; the collection stamps the embedder identity and refuses a mismatch
  (`vector.rs:7-9, 49-60, 833-850`; `crates/htui/src/concepts_worker.rs:1-8, 29-37`;
  `crates/htui-mcp/src/tools/search.rs:96-111`; `R-NF-2`, `docs/REQUIREMENTS.md:769-775`). No per-request
  client timeout was found in `client()` (`vector.rs:1010-1024`); qdrant-client defaults were not checked.
- **No semantic duplicate detection exists in code.** The RRF score is documented as comparable only
  within one search (`vector.rs:339-356`), so it cannot be a similarity threshold. The `duplicates` link
  kind is an ANA-35 design, not code. ANA-37 lists "triage and duplicate detection" as hoped-for uses
  (`HANDOFF.md:120-126`; `docs/requirements-review-2026-10-15.md:272-276`).
- ANA-37: the review wants Qdrant required "if it proves very beneficial" and lists "the R-ENT-13 memory"
  among the benefits to measure. Until it decides, Qdrant is optional. ANA-37's own result is not on this
  branch (see section 9 on an unverified recollection).

### 2.5 The MCP and writer seam

- Eight MCP tools today, scoped by session token (run, step, item, project never come from arguments),
  item writes fenced by the run lease, agent text scrubbed fail-closed. `note_add` writes a note
  immediately (16 KiB cap, `via_step_id`), `document_write` writes the step's output on its own item,
  `item_status` writes only a "status request" note, `item_link` proposes edges. `search_concepts` is
  project-scoped, limit 1..=20, types item/document/requirement. A fresh chat gets only `box_profile` and
  `search_concepts`. Not built: `item_create`, read tools for items/documents/requirements, the planner
  tools, workspace-wide search (`docs/htui-mcp.md`, tools table and scope sections, approximate lines
  48-240; `docs/ANA-34.md:162-194`; `docs/REQUIREMENTS.md:568-577`).
- **The draft/accept mechanism does not exist.** ANA-34 stores proposals "as drafts no transition, link,
  prompt or import reads until a person accepts them"; MOD-135 builds "a draft store for proposed items no
  reader consumes until accepted, shared with R-MCP-2's item_create"; MOD-136 gives coordinator turns tools
  that write into it (`docs/ANA-34.md:528-541, 662-670`; `HANDOFF.md:418-436`). No `draft` string appears in
  any migration. The only agent-proposes-person-accepts code today is in-memory editor-panel drafts
  (`crates/htui/src/agent_settings.rs:317, 405`). Requirement writes are the person-gated precedent
  (`proposed_by_step_id` NULL marks a human). Under the 2026-10-19 decision memory has no draft state and no
  longer needs this mechanism; MOD-135 stays relevant to `item_create` and coordinator proposals only.
- Agent step outputs (documents) already flow into later prompts without acceptance, as item work under
  `R-ID-6`. Close-out is a pure formatter and "a model never goes inside close-out" (MOD-98, ANA-2
  invariant 3; `HANDOFF.md:593-604`). The close-out checklist (ANA-34 section 7.6, MOD-135) is the natural
  accept touchpoint for a "memory candidates" row; it does not exist yet (`docs/ANA-34.md:604-616`).
  Under v2 memory does not use that touchpoint: an extraction turn at run end writes entries, and the rule
  "a model never goes inside close-out" is kept by running it as a step before close-out, not inside the
  formatter (unverified how the engine would schedule such a step).

### 2.6 Where memory-like content lives in this repo's workflow today

All local, per-checkout or per-user, and not shared across workers: Claude auto-memory (41 files, hand
indexed in `MEMORY.md`), Gortex remember/recall (private to an hr sandbox and deleted on purge), `.remember/`
and `.serena/memories` (git-ignored), and HANDOFF/DECISIONS/`docs/decisions` write-ups, which are in git but
record decisions only (`.claude/skills/handoff-run/SKILL.md:186-188`; `.gortex.yaml`). Staleness is already
visible there: line-number citations rot within a day or two (`docs/ANA-34.md:263-264, 756` cite
`HANDOFF.md:160-166` and `REQUIREMENTS.md:221-225`; the live ones are `HANDOFF.md:127-135` and
`REQUIREMENTS.md:270-274`); an auto-memory note says two agents carry `model: opus` though neither agent
file has a `model:` line now; another is frozen at 2026-10-17. A pin and a "last confirmed" mark would have
caught these. A Claude Code session started by htui does not load the user's settings, hooks or skills, but
the repository's `CLAUDE.md` and the user's MCP servers stay on (`docs/htui-mcp.md:389-428`); whether
Claude's own auto-memory loads there is undocumented. Memory therefore rides htui's prompt or MCP, not
vendor memory files.

## 3. Prior art

### 3.1 Coding harnesses

All claims from vendor docs unless noted. Pattern: a person-written, version-controlled layer beside an
agent-written layer, and vendors say the agent layer must not carry mandatory rules.

- **Claude Code.** CLAUDE.md is the person's; auto memory is Claude's, per repository, machine-local, an
  index plus topic files, first 200 lines or 25 KB of the index loaded; typed frontmatter; a `modified`
  timestamp "to show how current the fact is"; no expiry, no code pinning, no approval step (audit via
  `/memory`). https://code.claude.com/docs/en/memory
- **GitHub Copilot Memory.** The agent writes via a tool: subject, fact, `path:line` citations, reason.
  Verification is just-in-time: before use the agent re-reads the cited locations on the current branch;
  a contradicted memory is re-stored corrected, a verified one is re-stored, refreshing its timestamp.
  GitHub rejected an offline curation service as too costly. Docs add: unused entries deleted after 28
  days (the blog has no TTL; whether both describe the same version is unknown); repo facts only from
  contributors with write access. Seeded false memories were caught ("self-healed"). Reported: coding-agent
  PR merge rate 83% to 90%, review precision +3%, recall +4% (vendor-reported, public preview). The check
  is prompted behaviour, not a hard-coded check.
  https://github.blog/ai-and-ml/github-copilot/building-an-agentic-memory-system-for-github-copilot/ ,
  https://docs.github.com/en/copilot/concepts/agents/copilot-memory
- **Cursor.** Generated Memories (background model proposes, person approves, added in 1.2) were removed in
  v2.1 (Nov 2025); persistence is Rules only (`.cursor/rules/*.mdc`, `AGENTS.md`). The history comes from
  forum and blog sources, not a primary doc. https://cursor.com/docs/context/rules ,
  https://forum.cursor.com/t/agents-have-lost-access-to-memory-capability/143310
- **Windsurf.** Memories written by Cascade on its own judgment, workspace-scoped, local-only, retrieved
  when judged relevant, no approval; the docs point to Rules/AGENTS.md for durable knowledge.
  https://docs.devin.ai/desktop/cascade/memories
- **Cline Memory Bank.** A convention: six markdown files the agent maintains and must read every task.
  Nothing enforces it; no expiry or pinning. https://docs.cline.bot/prompting/cline-memory-bank
- **Aider.** `CONVENTIONS.md`, person-written, read-only, no agent write path.
  https://aider.chat/docs/usage/conventions.html
- **OpenAI Codex.** AGENTS.md is person-written and size-capped (32 KiB); Codex memories are an opt-in
  background pipeline writing generated state to `~/.codex/memories/`, no approval, and the docs say they
  are a "helpful recall layer", mandatory rules belong in AGENTS.md.
  https://learn.chatgpt.com/docs/agent-configuration/agents-md ,
  https://learn.chatgpt.com/docs/customization/memories?surface=app
- **Gemini CLI.** Hierarchical GEMINI.md concatenated into every prompt; the agent saves facts by editing
  markdown. Approval, expiry and failure modes are undocumented; applicability is unclear after a vendor
  banner about a replacement CLI. https://geminicli.com/docs/tools/memory/ ,
  https://geminicli.com/docs/cli/gemini-md/
- **Devin Knowledge** (deprecated, migrating to Skills). Items have a trigger and short content; Devin
  proposes, a person must save, edit or dismiss; repo pinning (none, one repo, all); retrieved on demand;
  nothing refreshes automatically. https://docs.devin.ai/product-guides/knowledge
- **Augment.** Agent-generated memories originally piled up unseen; Memory Review (2025-09-08) shows
  "1 Pending Memory" and the person approves, edits or discards. https://www.augmentcode.com/blog/how-we-built-memory-review
  (Rules and guidelines details came from search summaries only.)
- **Serena.** Markdown memories both person and agent write, project or global; not injected, the agent
  gets names and reads deliberately; `serena memories check` reports missing `mem:` links only, a link
  check, not a code-truth check. https://oraios.github.io/serena/02-usage/045_memories.html
- **Gortex remember/recall.** Public docs are thin: symbol-anchored development memories and session
  notes; writer, storage, staleness and injection undocumented in the pages fetched.
  https://gortex.dev/ (search summary), https://raw.githubusercontent.com/zzet/gortex/main/docs/mcp.md

Lessons: (a) a person gate or reviewability has been the direction of travel (Cursor 1.2, Augment Memory
Review, Devin), and the unreviewed variants accumulated low-quality entries; (b) agent-written free-form
memory has a record of being retired in favour of rules, skills and person-reviewed files (Cursor removed
it, Windsurf limits it to the legacy agent, Devin Knowledge is deprecated), which is the same
over-engineering risk ANA-1 was dropped for; (c) injection comes in three styles, an eager small index
(Claude, Copilot), lazy retrieval by trigger (Windsurf, Devin), and names-only plus deliberate read
(Serena); the cautionary cases are Cline's read-everything and Claude's silent truncation past 200 lines;
(d) local memory does not travel across machines or sandboxes, while htui's Postgres store does;
(e) maintainer decision, 2026-10-19: htui takes the unreviewed-but-guarded route that Windsurf, Codex
memories and Claude auto memory took, not the person-gated one. The accumulation of low-quality entries
seen in (a) is therefore the failure to design against (section 6), and the automatic safeguards of Option
E-prime stand where the vendors' gate stood.

### 3.2 Memory systems and frameworks

- **Mem0.** LLM extracts facts per message pair; an LLM tool call chooses ADD/UPDATE/DELETE/NOOP against
  the top-10 similar memories (https://arxiv.org/html/2504.19413). Current docs say it is ADD-only with
  optional `expiration_date` hiding expired memories from search, and warn that mixing modes can store a
  fact twice (https://docs.mem0.ai/core-concepts/memory-operations/add). Qdrant is a supported backend
  (https://docs.mem0.ai/components/vectordbs/dbs/qdrant). In its own paper's LoCoMo table, full context
  (72.90) beats every memory system; memory wins on latency and tokens (about 1.8k vs 26k), not accuracy.
- **Zep / Graphiti.** Bi-temporal knowledge graph; an LLM sets an older edge's `t_invalid` when a new one
  contradicts it; many LLM calls per episode (https://arxiv.org/html/2501.13956 ,
  https://github.com/getzep/graphiti). LoCoMo numbers between Zep and Mem0 are disputed by both vendors
  (https://www.getzep.com/blog/lies-damn-lies-statistics-is-mem0-really-sota-in-agent-memory/); an
  independent audit (non-peer-reviewed) found 6.4% of LoCoMo answer keys wrong.
- **Letta / MemGPT.** Core memory blocks (label, value, char limit, optionally read-only) always in the
  prompt, edited by the agent or developers (https://docs.letta.com/guides/agents/memory-blocks). Letta
  says a plain filesystem agent scored 74.0% vs Mem0 graph 68.5%, its own run (https://www.letta.com/blog/benchmarking-ai-agent-memory).
  MemGPT tier details are recalled, not read.
- **LangMem** (collection vs profile, hot-path vs background writes;
  https://langchain-ai.github.io/langmem/concepts/conceptual_guide/), **A-MEM** (LLM-linked notes that
  rewrite older notes; lowest in Mem0's table; https://arxiv.org/abs/2502.12110), **Cognee** (remember/recall/
  improve/forget; https://docs.cognee.ai/core-concepts/overview), **MemOS** (https://arxiv.org/abs/2507.03724).
  Write paths and backends for the last two were not stated on the pages fetched.
- **Anthropic memory tool.** Client-side files under `/memories`; the app owns storage and may expire
  unaccessed files. https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool
- **Entry model evidence.** A controlled ablation (single-author preprint) found verbatim chunks 43.9% vs
  extracted typed artifacts 28.0% on LoCoMo; adding artifacts beside chunks kept accuracy, replacing chunks
  lost it (https://arxiv.org/abs/2601.00821). LongMemEval recommends facts as extra index keys with raw text
  kept (https://arxiv.org/abs/2410.10813). So the evidence does not support replacing text with extracted
  atomic facts. On long histories a full-history baseline ties the best memory system (https://arxiv.org/abs/2607.21962,
  single paper, simulated users).
- **Conflict handling.** On MemoryAgentBench single-hop fact consolidation, Mem0 scored 18% and Graphiti 7%
  against BM25 at 48-56%; a deterministic newest-wins recipe reached 82-93%, though a critique says that
  split may not measure forgetting (https://arxiv.org/html/2606.01435v1 , https://arxiv.org/html/2610.09193 ,
  https://huggingface.co/papers/2507.05257; second-hand numbers).
- **Benchmarks are not a verdict.** Vendor figures are self-reported and not independently reproduced
  (https://mem0.ai/blog/ai-memory-benchmarks-in-2026). No published head-to-head favours a memory system
  over a plain-text baseline for a workload like htui's: code-bound, curated, human-reviewed, small.

### 3.3 Freshness and poisoning

- **Citation re-read (Copilot).** The best precedent: every entry carries citations re-checked at use.
  Only the shape transfers; Copilot's check is prompted LLM behaviour, htui needs code.
- **Swimm.** Couples docs to code by snippets, paths and "Smart Tokens", checked in IDE, on pull and in CI,
  with auto-sync repair. What is pinned, whether the check is deterministic and the false-positive rate are
  not public (patented). https://swimm.io/blog/sync-dont-sink-why-we-built-swimm-for-dev-teams ,
  https://swimm.io/blog/feature-updates
- **Multi-selector anchoring.** Hypothesis and W3C Web Annotation store several selectors (offset, quote
  with prefix and suffix, fuzzy) and re-attach in order of cost; fuzzy re-attachment can attach to the
  wrong place, and Semiont's stricter variant marks anything but a verbatim match low-confidence.
  https://web.hypothes.is/blog/fuzzy-anchoring/ , https://www.w3.org/2014/04/annotation/slides/RobustAnchoring.pdf ,
  https://github.com/The-AI-Alliance/semiont/blob/main/docs/protocol/W3C-SELECTORS.md
- **ADR practice.** Proposed, Accepted, Deprecated, Superseded-by; an accepted record is replaced, not
  edited; nothing ties it to code, so staleness depends on human review.
  https://www.archyl.com/blog/architecture-decision-records-complete-guide
- **Doc-rot research** is dominated by LLM or learned classifiers; one reports plain LLM checks flag over
  90% of functions as inconsistent (https://arxiv.org/pdf/2511.00215). The deterministic strand is name
  resolution of code names found in prose (DOCREF, known from search summaries only).
- **Stale memory is worse than none (abstract-only sources).** TEPA reports append-only memory 0.203, no
  memory 0.298, with revocation 0.950 (https://arxiv.org/abs/2608.07429); MERIT reports agents use a
  correctly retrieved updated value about 55% of the time and embedding retrieval is unstable
  (https://arxiv.org/abs/2609.05441). VibeMemBench: verified relevant experience gives +1.1 to +4.5 points
  on 4 of 5 solvers, but eleven of twelve self-built memory pairings fail to beat memory-off
  (https://arxiv.org/abs/2609.23570); DreamBench-SWE: deterministic verbatim event memory 82/180 vs 21/180
  with none (https://arxiv.org/abs/2608.20664).
- **Poisoning.** MINJA injects records through ordinary queries (over 95% injection success reported)
  because shared memory is retrieved into later contexts (https://arxiv.org/pdf/2503.03704); OWASP ASI06
  frames saved injected text as a standing instruction that re-fires in later sessions; provenance
  laundering lets LLM consolidation rewrite untrusted text so it looks like user history
  (https://arxiv.org/pdf/2607.29167). Recurring mitigations: write-time admission (including human
  approval), platform-kept provenance outside the model, retrieval-time filtering, TTL by source
  (secondary sources: https://www.emergentmind.com/topics/persistent-memory-poisoning). No source
  documents a human-accept flow end to end, and none documents a fully automatic design with the safeguards
  chosen here, so both the v1 accept gate and the v2 automatic safeguards are design choices, not
  precedents.
- **Qdrant and pgvector.** One collection with a keyword payload field and `is_tenant:true`, never a
  collection per scope (https://qdrant.tech/documentation/guides/multiple-partitions/); payload filters
  can carry status, commit SHA, validity windows (https://qdrant.tech/documentation/concepts/filtering/);
  Qdrant has no built-in dedup, a maintainer advises a per-dataset threshold
  (https://github.com/orgs/qdrant/discussions/3268); pgvector filters apply after the ANN scan, fixed by
  iterative scans since 0.8.0 (https://github.com/pgvector/pgvector). For a small scope-filtered corpus a
  btree on scope plus exact scan or full-text is plausibly enough (inference, not benchmarked).

## 4. Options

The five settle-points used throughout: **S1** who writes; **S2** how it stays fresh; **S3** how it meets
`R-ID-6`; **S4** how it meets Qdrant and `R-STO-8`; **S5** data model and prompt reach. Each option is also
compared against ANA-37 (does it need Qdrant required) and cost.

One classification decides S1 and S3 before any option can be judged: **is a memory entry item work or
bookkeeping?** Step documents flow into later prompts unaccepted because they belong to one item; a memory
entry outlives it and fires in every later run. Draft v1 recommended
treating entry creation as bookkeeping, so a model-authored entry reaches a prompt only after a person
accepts it. The maintainer's answer (2026-10-19) overrides that: entry creation stays outside the
deterministic bookkeeping rule as an explicit, owed exception to `R-ID-6`, and a model writes entries that
become active without approval. Option E keeps the v1 recommendation as the alternative; Option E-prime is
the v2 recommendation.

### 4.1 Option A - Summaries only, no new entity (status quo of the withdrawal)

Keep `R-ENT-13` withdrawn or satisfied by `summary` documents. Optionally make the close-out summary richer
(MOD-98's opt-in model-written summary phase, `HANDOFF.md:593-604`).

- S1: close-out code writes a commit table; a model-written summary is item work in an opt-in graph.
  S2: none; a summary is frozen at close-out. S3: clean for the table, MOD-98's phase is a model writing
  a document a later prompt reads, which is item work by today's reading. S4: summaries are already
  indexed as document points. S5: reaches a prompt only over `blocked_by`/`origin` edges within two hops.
- Does not meet the stated need: an unlinked item's gotcha never surfaces, and the summary has no prose
  today. Cost: nothing new. ANA-37: nothing to measure. This is the honest baseline against which the rest
  must be judged, and the only option the original withdrawal endorsed.

### 4.2 Option B - Person-written notes (entries a person writes, no agent writer)

A person writes entries directly. Home is either a project-owned (later workspace-owned) document of a
conventional kind (ANA-35 owner documents), or a skill today.

- S1: a person only. S2: person confirmation, version pin of the document; optionally the citation stamp
  below. S3: trivially clean, no model text. S4: owner documents index as their own points in ANA-35's v4
  collection with no extra work. S5: skills are protected and token-capped and need no prompt change; an
  owner document needs a new prompt section.
- Cost: lowest. No migration if it is a document kind; no draft store (MOD-135) needed. Weakness: depends
  entirely on a person remembering to write; this is the lane the maintainer's own workflow already shows
  failing silently (`docs/ANA-29.md:84-86`). ANA-37: Postgres-only, does not argue for Qdrant.

### 4.3 Option C - Agent-proposed entries a person accepts

An agent (run step, chat, later a coordinator turn) proposes an entry through an MCP tool into the MOD-135
draft store. A person accepts at the close-out checklist or in a pane. Nothing reads a draft.

- S1: agent proposes, person disposes; also a person directly. S2: needs a freshness field (see 4.5).
  S3: exactly the `R-ID-6` pattern ANA-34 already settled; stricter wording in `REQUIREMENTS.md:121-131` says
  drafts "are written only once a person accepts them", so where a draft sits before acceptance needs a
  ruling (question 2). S4: only accepted entries are indexed; the sync query filters on state.
  S5: needs the draft store, a state column, and prompt reach as in Option B.
- Cost: moderate, and **blocked on MOD-135**, a filed item with no schema. Risks: approval fatigue
  (Cursor), accumulation (Augment before Memory Review), poisoning if the gate becomes a rubber stamp.
  ANA-37: Postgres-only baseline works.

### 4.4 Option D - Atomic facts with importance restored (ANA-1 as written, modernised)

Revive `artifact` plus `artifact_link`, importance 1-10, recursive-CTE injection, perhaps with a model
extracting facts.

- S1: implicitly agents (`run_step_id` FK). S2: `commit_hash` flag only. S3: an agent writing straight into
  a prompt-visible store is the one option the contract rules out; a model extractor at close-out is
  excluded by MOD-98. S4: fits the requirement point pattern but dedup of extracted facts is the hardest
  Qdrant use to justify. S5: new tables, a new CTE, importance as an unmeasured retrieval heuristic.
- Evidence against: extracted atomic facts lose to verbatim text; a fact-link graph and LLM contradiction
  decisions score worst on conflict benchmarks; ANA-11 invariant 5 and the withdrawal precedent. Cost:
  highest (two tables, SQLite mirror, about eight store backends, CTE, scoring UI). ANA-37: raises
  Qdrant's apparent value only by adding a dedup workload nobody measured. Listed for completeness; not
  recommended.

### 4.5 Option E - Hybrid with a person gate (the v1 recommendation, now the alternative)

Verbatim entries in three lanes, deterministic freshness, Postgres-only prompt selection, Qdrant as an
add-on. Superseded as the recommendation by Option E-prime (4.6) but kept whole, because it is the
fallback if P7 shows that the automatic path does harm.

- **Entry:** subject, prose body, owner scope (project; workspace after the scope row exists), state
  (`draft`, `accepted`, `suspect`, `superseded`, `withdrawn`), optional citations, optional pin, provenance
  stamped by code (author or run, step, item, commit). No importance score and no fact-link graph. One
  deterministic `supersedes` pointer keyed on (scope, subject), confirmed by a person.
- **Home:** start as an ANA-35 owner document with a conventional kind (for example `memory`); add a
  dedicated table only if pin, state and key cannot sit in document columns. Honest risk: a document has
  no pin or state columns, so the cost may move there.
- **Writers (S1):** (1) a person in the TUI; (2) a person promoting an existing note (`note_add` exists
  today, so this has a useful first step before MOD-135); (3) an agent proposing through an MCP tool into
  MOD-135's draft store, accepted at the close-out checklist; later (4) coordinator turns drafting there.
  Close-out code lists the item's drafts for one confirmation but never extracts with a model.
- **Freshness (S2), computed by code only:** (a) a version stamp on whatever the entry cites (requirement,
  document or item), reusing `makes_suspect` and `reconfirm`; (b) for git projects, an optional pin of
  (repo, commit, cited paths or symbols) taken from the writing step's `after_hash`, checked by a **sweep**
  that compares "cited paths changed since the pin" and stores a suspect flag; (c) a "last confirmed" date
  as a display aid; (d) disuse expiry only as a display flag, never a silent delete. A suspect entry is
  shown with a marker or held back from prompts until a person re-confirms, amends or supersedes it. Pin to
  paths or symbols, never line numbers.
- **Prompt reach (S5):** a new internal section (like Persona/Output, no template names it), filled by a
  Postgres query in `phase_spec` and handed to `assemble` as a pre-resolved `PromptSpec` field; accepted
  non-suspect entries by owner scope, deterministic order and tiebreak, hard token cap, one selection per
  run (a run-start snapshot) so siblings and retries see the same set; ranked after Excerpts and before
  Upstream in the trim order, unprotected; chosen entry ids and versions recorded in `trim_record`
  (`RECORD_VERSION` 4 to 5); entries rendered as quoted data, labelled with their source and trust, not
  as directives. Overflow is pulled by the agent through a search tool. Amends `R-PRM-1`, `R-PRM-3` and
  CONCEPTS "Bounded context".
- **Qdrant (S4):** one more point type in the single collection, built like the requirement path (one short
  point per accepted entry, rebuilt when `version` differs, deleted when the row is gone), delivered with
  ANA-35's planned collection rebuild so there is one reindex; extend `search_concepts` types rather than
  add a second search tool; display-only "possible duplicate" hints at accept time. A dedup hint needs a new
  `VectorStore` method returning raw dense cosine, because the existing RRF score cannot be thresholded; the
  0.98 and 0.82 reference thresholds in the literature are for other embedders and cannot be reused.
- **Coordinator:** a coordinator turn's seed reads accepted entries from rows, without the `R-PRM-3`
  ladder; a project with no coordinator agent loses nothing, since entries are listable and the checklist
  is deterministic.
- Cost: moderate and phased. P1 needs no draft store, no workspace row and no git read. ANA-37: works
  Postgres-only and hands ANA-37 a measurable (paraphrase recall and duplicate-hint quality on real
  entries) without presuming its answer.
- **Trade-off against E-prime:** the safest poisoning posture (no unaccepted text is ever read) and a
  clean `R-ID-6`, bought with a person's review time. Its weaknesses are the ones the maintainer rejected:
  approval fatigue and accumulation of unreviewed drafts (Cursor, Augment before Memory Review), a rubber
  stamp gate whose value depends on a review volume nobody estimated, and a hard dependency on MOD-135,
  a filed item with no schema. It also never captures a lesson nobody thinks to propose, which is the
  case the new `lesson` kind exists for.

### 4.6 Option E-prime - Automated hybrid (recommended, v2)

Option E's entry model, freshness and prompt reach, with the person gate replaced by a model writer and a
deterministic pipeline of automatic safeguards. Decided by the maintainer on 2026-10-19; the safeguards are
not approval gates and nothing waits for a person.

- **Entry:** subject, prose body, **kind** (`fact`, `decision`, `gotcha`, `lesson`), owner scope (project,
  workspace, **global**), state (`active`, `suspect`, `superseded`, `retired`; no draft state), evidence
  (citations of what was observed: path, error text, step), optional pin, a pinned-as-trusted flag set by a
  person, deterministic usage counters, and provenance stamped by code (run, agent, item, commit; for a
  global entry also the agent, tool or CLI version it was learned on). No importance score and no
  fact-link graph. One deterministic `supersedes` pointer keyed on (scope, subject), now set
  automatically at write time.
- **The `lesson` kind:** recurring errors, optimizations already done, things that improve usage over time.
  A lesson is not tied to a workflow or an item and is owned by a project or a workspace (or, after
  promotion, global); the run it came from is provenance only, never an owner.
- **Global scope:** a third owner above workspace, holding lessons "that can be generalized, and gotchas
  that can happen in any other project/workspace": tooling and general practice, not project facts. It is
  a third hidden scope row, the same pattern as the workspace scope row of ANA-35 (`docs/ANA-35.md:937-947`),
  and depends on the MOD-147 work. Whether the row pattern carries over unchanged to a row with no
  workspace, project or repo is not verified.
- **Home:** start as an ANA-35 owner document with a conventional kind (question 3, accepted as
  recommended), and move to a dedicated table only if forced. Honest risk, larger than in Option E: state,
  pin, trusted flag, usage counters, kind and a three-level scope do not obviously fit document columns, so
  P1 should settle table or document before anything is built (inference, not checked against the
  ANA-35 document shape).
- **Capture points (writer = a model):** (1) **run end**: an extraction turn by the configurable light
  model (`R-ORCH-1`; it spends the user's plan) over the run's own evidence, meaning errors seen, retries,
  fixes and measured wins; (2) a **deterministic detector** over run records for repeated error
  signatures, retries, timeouts and outlier cost, which triggers or feeds that extraction. The first task
  of the detector's phase is to verify what error and retry detail run records store today; that is **not
  yet verified**. A person may also write an entry directly (manual write), and a note can be promoted.
- **Write path (deterministic code around the model):** schema-validated output; a per-run write cap;
  scrub (`R-ID-7`) before persist, failing closed; provenance stamped by the platform, which the model
  cannot set; the evidence must cite something observed in the run (a path, an error text, a step), and an
  entry without it is rejected. Each control answers a recorded risk: the provenance-laundering and
  standing-instruction findings (section 3.3) argue for platform-kept provenance and write-time admission,
  and the self-built-memory results (eleven of twelve pairings failing to beat memory-off) argue for
  admitting less rather than more.
- **Dedup and supersede at write time, automatic:** the same (scope, subject) key, or high similarity,
  replaces (supersedes) the older entry instead of appending. The Postgres-only fallback is the subject
  key plus full-text. Qdrant, when present, improves recall through a **raw-cosine** query (the existing
  RRF score cannot be thresholded; the 0.98 and 0.82 reference thresholds in the literature are for other
  embedders and do not transfer). This makes ANA-37's measurement more valuable, and memory still works
  with Qdrant absent. The literature points the same way: replacing beats appending, and a deterministic
  newest-wins recipe beat LLM contradiction decisions (section 3.2).
- **Freshness (S2), computed by code:** version stamp on cited rows, optional commit-and-path pin for git
  projects, checked by a sweep that stores a suspect flag. A suspect entry is **held back from prompts**,
  and an automatic **re-validation turn** (triggered by the sweep, reading the cited paths) either
  re-confirms (new stamp), amends (a new superseding version) or retires it. Unresolved suspect entries
  auto-retire after a configurable period. A global lesson is stamped with the agent, tool or CLI version
  it was learned on and goes suspect when that version changes, since no repo pin applies. The model never
  decides that an entry is stale; code does, and the model only re-reads what code flagged.
- **Prompt reach (S5), unchanged in shape:** a new internal section filled by a Postgres query in
  `phase_spec`; active non-suspect entries by owner scope, deterministic order and tiebreak, a hard token
  cap, one run-start snapshot, ranked after Excerpts and before Upstream, ids and versions recorded in
  `trim_record`, entries rendered as quoted data labelled with source and trust. New: deterministic usage
  counters (injected into N steps; of those, how many ended done versus failed) used to rank demotion and
  retirement, with no model. The global section has its own small cap and ranks below project and
  workspace.
- **Global promotion, automatic through a deterministic gate:** a generalization check (a model decides
  "applies beyond this project") followed by a deterministic validator that rejects any text containing
  project or workspace-specific tokens (repo, project and workspace names and keys, absolute paths,
  hostnames, secret patterns, URLs of private forges) and re-runs scrub. Failing either keeps the entry at
  its original scope. An agent may not target global directly; promotion is a separate pipeline step.
  `R-PRM-1`'s "reads stay inside the active workspace" needs one clause for global tooling lessons.
- **Human visibility without gating:** a TUI list of recent automatic entries with edit, retire,
  pin-as-trusted and a "why is this here" view (provenance, evidence, injected-in runs); a settings switch
  that turns automatic capture and global promotion off per cascade level (Settings, workspace, project),
  default on, per the maintainer.
- **Qdrant (S4):** one more point type for entries in the single collection, built like the requirement
  path and delivered with ANA-35's planned collection rebuild; `search_concepts` types extended; raw-cosine
  dedup as above. Qdrant never decides what enters a prompt or whether an entry is fresh.
- **Coordinator:** a coordinator turn's seed reads active entries from rows without the `R-PRM-3` ladder;
  MOD-136 is needed only for that seed, not for memory itself.
- **Accepted risks, stated plainly:** cross-project leakage of anything the validator misses; a larger
  poisoning blast radius (one bad global entry reaches every project); unreviewed standing instructions;
  a model call on every run. Section 6 lists each with its mitigation.
- Cost: moderate-to-high and phased. P1 and P2 are the committed slice. New recurring cost is model tokens
  on every run plus re-validation and generalization turns; per-run size is not measured (section 9.2). It
  needs no draft store (MOD-135 no longer blocks memory), but needs MOD-147 for workspace and global scope.
  ANA-37: works Postgres-only and hands ANA-37 a measurable (raw-cosine dedup recall against the
  subject-key plus full-text fallback) without presuming its answer.

### 4.7 Comparison against the settle-points, R-ID-6, ANA-37 and cost

| | A summaries only | B person-written | C agent-proposed, accepted | D atomic facts + importance | E hybrid, person gate (v1) | E-prime automated hybrid (v2, recommended) |
|---|---|---|---|---|---|---|
| S1 who writes | close-out table; opt-in model phase (MOD-98) | a person | agent proposes, person accepts | agents (implicit) | person, promoted note, accepted agent draft | a model (light model) through a validated pipeline; a person may also write |
| S2 freshness | none | person confirm; version pin | needs a field | `commit_hash` flag only | version stamp + optional pin by sweep; person re-confirm | same computed freshness; automatic re-validation turn; auto-retire; global lessons tied to tool version |
| S3 R-ID-6 | clean | clean | clean once draft store exists | violated unless gated | clean; one-sentence ruling needed | explicit amended exception (owed edit) |
| S4 Qdrant / R-STO-8 | document points already | via owner-document points | accepted-only point type | dedup hardest to justify | add-on point type; hints; Postgres baseline | add-on point type; raw-cosine dedup; Postgres fallback (subject key + full-text) |
| S5 model and prompt | edge-driven, 2 hops | skill or new section | new section + draft store | 2 tables + CTE | owner doc or small table + internal section | owner doc or table + internal section; usage counters; global section with own cap |
| Meets the need | no (unlinked gotchas lost) | partly (depends on discipline) | yes | yes, over-built | yes, smallest slice first | yes; also captures lessons nobody proposes |
| Poisoning exposure | none | none | gated by a person | high | gated by a person | no human gate; contained by write-path safeguards, global gate, visibility |
| Needs Qdrant required (ANA-37) | no | no | no | pushes toward it | no; Qdrant optional | no; Qdrant improves recall only |
| Cost | nil | lowest | moderate, blocked on MOD-135 | highest | moderate, phased | moderate-to-high: model tokens on every run; MOD-147 for workspace and global |
| Withdrawal risk | none | low | low | repeats ANA-1 | low if scores and links stay dropped | medium: more machinery than E; scores and links stay dropped |

## 5. Evaluation

Reading the table with the critic's caveats:

- **A is the baseline and it fails the stated need** on a structural point (edge-driven reach, no prose
  writer), not on a measured failure. No evidence exists that a missing gotcha caused a failed step in the
  product; ANA-29's ten frictions concern the maintainer's development setup. The reopening is a
  maintainer instruction without new evidence, so the case for any option beyond A rests on structure and
  on the design being cheap enough to test.
- **B alone is the cheapest real test** of whether memory pays off; E's phase 1 is B plus the prompt
  section. B alone is also the lane that, in this repo's workflow, already fails silently, which is why C
  matters.
- **C is the only writer path that scales** and the only one with a poisoning surface; it needs MOD-135 and
  a ruling on where drafts sit before acceptance.
- **D is dominated.** The evidence (verbatim beats extracted facts; replacing beats appending; LLM
  contradiction decisions perform worst) and the project's own precedent (ANA-11 invariant 5) point away
  from it.
- **E was B then C then freshness automation**, in the order the dependencies allowed. It is no longer
  the recommendation, but it is the fallback if the automatic path proves harmful.
- **E-prime is E with the gate replaced by safeguards**, and the evidence cuts both ways. For it: C's
  approval burden and the silent failure of person-only lanes (ANA-29), and the vendors that shipped
  unreviewed memory (Windsurf, Codex, Claude). Against it: the self-built-memory results (most pairings
  fail to beat memory-off; stale or wrong memory reported worse than none) and the poisoning literature,
  which treats a saved entry as a standing instruction. So E-prime is the recommendation because the
  maintainer chose it, with its weakest points named: nothing outside the safeguards stops a wrong entry,
  and the safeguards are untested on htui data. Each phase stays independently testable, P1 and P2 depend
  on nothing unbuilt except the table-or-document choice, and P7 exists to catch harm.
- **Benefit evidence is thin and mixed.** Vendor-reported Copilot gains; independent coding-agent
  benchmarks show verified experience helping by a few points while self-built memory mostly does not;
  stale memory is reported worse than none. This argues for verified-only injection and a small design,
  and for measuring on htui's own data before making anything "must" beyond P1 and P2. The evaluation plan
  is P7 (question 8, as adjusted). Automatic capture raises the stakes of that plan: harm from wrong
  entries is the measure that decides whether the gate-free design stays.

## 6. Risks

**Staleness.** The dominant risk and the reason freshness is a settle-point.
- A suspect entry that still reaches a prompt is worse than no entry. Mitigation: suspect entries are held
  back from prompts, never silently injected; the stamp check is code, and an automatic re-validation
  turn re-confirms, amends or retires them.
- The commit pin is **not available everywhere**: folder projects have no hashes; no branch name is stored
  per run or item; rebased or squashed pins become unreachable; htui writes nothing into a managed repo, so
  the check reads the repo read-only on the executing box; remote targets (ANA-42) have no repo state at
  the host. The version-stamp mechanism works for every project kind and is the baseline; the pin is an
  optional add-on for git projects.
- The freshness check **cannot run inside prompt assembly**: `assemble` is pure (no I/O, no repo access) and
  fan-out siblings need byte-identical prompts. It runs in a sweep (or at run start) and is stored as a
  flag; the selection reads the flag. The returns contradicted each other here (a "just-in-time at
  injection" model against the pure assembler); this design resolves it by precomputing.
- Citations by line number rot within days (observed in this repo); pin to paths or symbols.
- A path-intersection check is conservative: a busy file flags many entries (false positives), never
  misses a touch. A content or symbol hash would cut false positives but is a later refinement.
- Selecting by time (expiry, "last confirmed") must be resolved by the caller and recorded, never inside
  `assemble`. The auto-retire period for unresolved suspect entries is applied by the sweep, not at
  selection time.
- The re-validation turn is a model reading code the sweep flagged; it can wrongly re-confirm. Mitigation:
  it must cite what it re-read, the new stamp is code-written, and a re-confirmed entry that later ends
  runs in failure loses rank through the usage counters.

**Memory poisoning, with no human gate.** A saved entry is a standing instruction that fires in every
later run, so the attack is temporally decoupled (ASI06, MINJA), and under v2 nothing a person does sits
between a model's text and a later prompt. The gate of v1 is gone, so each mitigation below is a control in
code. Mitigations: provenance and trust stamped by the platform, outside the model (the model cannot set
it, which answers provenance laundering); evidence that must cite something observed in the run, so
a claim with no trace is rejected; schema validation, a per-run write cap and scrub on write (`R-ID-7`);
entries rendered as quoted data with a source and trust label, not as directives; writes only through
the pipeline, for a run the platform knows (run-scoped provenance, `R-MCP-1`), never a free tool call into
the store; supersede
rather than edit, keeping the old entry for audit; usage counters that demote entries whose runs fail;
the suspect state and re-validation; and human visibility (list, "why is this here", retire, per-level off
switch) so a person can find and kill a bad entry after the fact. Residual risk: an entry that passes every
control and is wrong or hostile stays active until freshness, counters or a person remove it. The
text a run observes (tool output, repository text) is untrusted input to the extraction turn, and whether
the evidence rule is enough against an injected string in an error message is not verified.

**Unreviewed standing instructions.** Even a correct entry is an instruction no person read. Mitigation:
the quoted-data rendering, the trust label, the small caps, and the visible "why is this here" view. The
prompt tells the agent an entry is a note from earlier runs, not a rule; whether agents honour that is
unmeasured (P7).

**Model cost on every run.** The light model spends the user's plan: an extraction turn at run end, a
generalization check per promotion candidate, and a re-validation turn per suspect entry. Per-run token
cost is not measured. Mitigations: the per-run write cap, extraction fed by the detector's evidence rather
than whole transcripts, a skip for runs with nothing observed (a proposal, to confirm), and the per-level
off switch. The cost is paid whether or not memory helps, which is one more reason P7 gates the "must".

**Global leakage.** A global entry is read by every project, so anything the validator misses leaks across
projects and workspaces, and one bad global entry reaches all of them (a blast radius larger than any
project's). Mitigations: an agent cannot target global; promotion is its own step through a model
generalization check and then a deterministic validator that rejects project and workspace-specific tokens
(names and keys, absolute paths, hostnames, secret patterns, private-forge URLs) and re-runs scrub; a
failure keeps the entry at its scope; a small global cap, ranked below project and workspace; stamping
with the tool version and suspecting on change; a switch to turn global promotion off per level. Residual:
the validator's recall for project-specific tokens is not measured (section 9.2), and `R-PRM-1`'s "reads
stay inside the active workspace" needs one clause so global tooling lessons are not a breach.

**Runaway growth.** Automatic writes with no review can pile up entries nobody reads. Mitigations: dedup
and supersede at write time, the per-run write cap, auto-retire of unresolved suspect entries, usage
counters that rank unused or failing entries for retirement, and the prompt caps (the store may grow, the
prompt may not). Residual: the store itself is unbounded without a retention rule (a residual question).

**Self-reinforcing wrong lessons.** A wrong lesson injected into later runs can cause the very errors that
the detector then reads as new evidence, so the store reinforces itself. Mitigations: evidence must cite an
observation in the run, not a prior entry; entries injected into a run are marked in the run record so the
extraction turn can discount evidence from runs the entry shaped (a proposal, unverified); usage counters
demote an entry whose injected steps end failed; a person can retire it. Residual: counters are
correlation, not cause.

**Qdrant dependence.** Retrieval quality is the weak spot in the literature (embedding retrieval unstable,
most self-built memory pairings not beating memory-off), and Qdrant is optional and fails open until
ANA-37 decides (`R-NF-2`, MOD-34/50/68). Therefore: prompt selection and freshness never read Qdrant; the
Postgres row is canonical and the index rebuildable (`R-ID-3`, `R-STO-8`); Qdrant adds agent-pull search
and better dedup recall only. With Qdrant absent, dedup falls back to the subject key plus full-text and
paraphrased duplicates get through more often, which is one more input to ANA-37. A just-written entry is
not searchable, and not visible to the raw-cosine dedup, until the next sync (15 minutes on a worker) or
an on-demand index request, so two near-identical entries written in one window can both land. A worker with no keyring never serves search, so agent-pull from a
provisioned worker is unavailable even today. If ANA-37 makes Qdrant required, the fail-open plumbing (a
worker without a keyring never indexes) must change first; ANA-38 then only makes the index step
mandatory, with no design change.

**Other risks.**
- Over-engineering, the withdrawal's own objection: mitigated by dropping scores, links and the CTE, and by
  making P1 and P2 a use test. Real remaining exposure is larger than in v1: a new entity or document kind,
  a model writer, a detector, a sweep, a re-validation turn and a promotion pipeline, plus store-trait
  methods across about eight backends and a SQLite mirror migration, if a dedicated table is chosen.
- Workspace reach: `R-PRM-1` says reads stay inside the active workspace, but the engine is headless, holds
  no workspace and walks project-bounded; a project may sit in several workspaces. Workspace memory in a
  project step cannot be built until the scope row (MOD-147) exists, and the global scope row depends on
  the same work, so workspace scope is not in P1 and global scope is P6.
- Fan-out consistency: needs a run-start snapshot and a `trim_record` version bump; the digest would
  otherwise lie for `R-ID-5`.
- Approval fatigue is gone with the gate, but the visibility surface is not designed: the list, edit,
  retire, pin-as-trusted and "why is this here" views, and the per-level switches.
- Authorization: who may edit, retire or pin-as-trusted, and how concurrent edits behave (`R-ENT-10`
  compare-and-set applies by default). A pinned-as-trusted entry's exemptions are a residual question.
- Offline mirror: whether entries must be browsable offline (`R-STO-3`) is still uncovered and decides a
  SQLite migration.
- Documentation hygiene: ANA-34 still cites stale line numbers for this item.

## 7. Verdict and phasing

### 7.1 Recommended answer per settle-point

Revised on 2026-10-19 after the maintainer's answers.

**S1 Who writes.** A model: the configurable light model of `R-ORCH-1`, through a validated deterministic
pipeline. Capture points are the end of a run (an extraction turn over the run's own evidence: errors seen,
retries, fixes, measured wins) and a deterministic detector over run records that triggers or feeds that
turn. A person may also write, edit and retire entries in the TUI. The code that writes the close-out
summary still never uses a model; the extraction turn is a separate step. The v1 answer ("an agent only
proposes into a draft store, a person accepts") is superseded and kept as Option E.

**S2 Freshness.** Code decides, and nothing waits for a person: version stamp on cited rows (existing
pattern), optional commit-and-path pin for git projects checked by a sweep that stores a suspect flag,
suspect entries held back from prompts, supersede rather than edit. An automatic re-validation turn
(sweep-triggered, reading the cited paths) re-confirms with a new stamp, amends with a new superseding
version, or retires; unresolved suspect entries auto-retire after a configurable period. Global lessons
carry the agent, tool or CLI version they were learned on and go suspect when it changes. No model decides
that an entry is stale; a model re-reads only what code flagged.

**S3 R-ID-6.** An explicit, amended exception: memory writes are not bookkeeping a person gates and are
not forbidden; they are a named exception to "no LLM in bookkeeping". Everything else in `R-ID-6` stays: no
status transition, link, import or cache refresh reads model text, and state changes, dedup keys, freshness
and prompt selection are deterministic code. The amendment text is proposed in 7.3 and is maintainer-gated.
No draft state exists, so the "where does a pre-acceptance draft live" ruling falls away.

**S4 Qdrant / R-STO-8.** Memory works with Postgres alone (subject key plus full-text for dedup). Qdrant adds
a point type for entries, agent-pull search through the existing `search_concepts`, and a raw-cosine query
that improves dedup recall at write time; delivered with ANA-35's collection rebuild. Nothing about prompt
content or freshness reads the index. Ask ANA-37 to measure paraphrase recall and dedup quality on real
entries.

**S5 Model and prompt reach.** Entries are verbatim prose with a subject, kind, scope, state and stamp; home
is an ANA-35 owner document kind to start (a table if forced, decided in P1), with a third, global owner
scope added. Prompt reach is a new internal section filled in `phase_spec`, active non-suspect entries
only, capped, snapshotted at run start, trimmed after Excerpts and before Upstream, recorded in
`trim_record`; a global section with its own small cap ranks below project and workspace; deterministic
usage counters drive demotion; the rest by agent pull. Not brought back: importance scores, the link graph,
recursive-CTE injection, model-ranked selection.

On ANA-1's atomic facts specifically: neither as they were nor summaries alone. Keep summaries; add
automatically written entries beside them.

### 7.2 Proposed phases (not filed; IDs are minted only after the maintainer accepts)

| P | Subject | Scope | Blocked on |
|---|---|---|---|
| P1 | Entry store and prompt section | Entry store (an owner document kind or a table, decided in question 3 at the start of the phase) with kinds including `lesson`, states `active`, `suspect`, `superseded`, `retired`, TUI list, edit and retire, and manual write; a prompt section filled by Postgres-only selection in `phase_spec` with a cap, a run-start snapshot, a trim rank and a `trim_record` bump; `R-PRM-1`/`R-PRM-3`/CONCEPTS amendments | project scope: none beyond ANA-35's owner-document work if a document kind is used (confirm which owner-document pieces project scope needs); not MOD-135; not MOD-147 |
| P2 | Automatic capture at run end | Extraction turn by the light model over the run's evidence; write-path safeguards (schema validation, per-run write cap, scrub before persist, platform-stamped provenance, evidence must cite the run); dedup and supersede at write time (subject key plus full-text); deterministic usage counters; per-level capture switch | P1; the `R-ID-6` amendment (maintainer-gated) before it is built; MOD-147 only for workspace-owned entries; MOD-136 only for the coordinator seed reading entries |
| P3 | Lesson detector over run records | Deterministic detector for repeated error signatures, retries, timeouts and outlier cost that triggers or feeds extraction. First task: verify what error and retry detail run records store today (not yet verified) | P2; the result of that verification |
| P4 | Freshness sweep, pins and re-validation | Version-stamp suspect flag; optional pin (repo, commit, paths or symbols) from the writing step's `after_hash`, git projects only; automatic re-validation turn; auto-retire of unresolved suspect entries; pin-as-trusted; the "why is this here" view | P1; P2 for the re-validation turn |
| P5 | Qdrant point type and raw-cosine dedup | Memory point type, active entries only, rebuilt on version change; `search_concepts` type extension; a raw-cosine `VectorStore` query for write-time dedup; rides ANA-35's planned collection rebuild; informed by ANA-37 | P1, P2; ANA-35's collection v4 work; MOD-112 for read tools |
| P6 | Global scope and automatic promotion | A third hidden scope row (global), same pattern as ANA-35's workspace scope row; the promotion pipeline (generalization check, then the deterministic validator and scrub); tool-version stamping and suspect-on-change; a small global prompt section; the `R-PRM-1` clause; the per-level promotion switch | MOD-147 work (scope row pattern); P2; P4 for suspect-on-change |
| P7 | Measurement on real htui steps | Before/after on real steps, recall of known gotchas, harm from wrong entries, FTS versus Qdrant; decides whether `R-ENT-13` stays "must" beyond P1 and P2; feeds ANA-37 | P1, P2; ideally P3, P5, P6 |

`R-ENT-13` priority: "must" for P1 and P2; "later" for P3..P6 until P7 reports. P1..P7 are a proposal and no
IDs are minted; the number may change at filing, as ANA-34's P1..P4 did before they became MOD-133..136.
MOD-135 (the draft store) no longer blocks memory; MOD-147 blocks workspace and global scope; MOD-136 is
needed only for the coordinator seed.

### 7.3 Requirement edits owed (all maintainer-gated; this item edits none)

- `R-ENT-13`: replace "reopened, pending ANA-38" with the settled text: kinds (`fact`, `decision`,
  `gotcha`, `lesson`), scopes (project, workspace, global), writers (the light model through a validated
  pipeline; a person may also write), states, the priority split, and that product memory is separate from
  the development workflow's own memory.
- `R-ID-6`, proposed amendment text: "Exception (ANA-38, 2026-10-19): a memory entry (R-ENT-13) may be
  written by the light model (R-ORCH-1) without a person's acceptance, through a deterministic write path
  (schema-validated, capped per run, scrubbed per R-ID-7, provenance stamped by the platform, evidence
  drawn from the run). An active entry may reach a prompt, the index and the coordinator seed. Nothing else
  changes: no status transition, link, import or cache refresh reads model text, and an entry's state,
  dedup key, freshness and selection are decided by code." Also state that the same configurable light
  model serves this use, a third beside workflow choice and draft rewrite.
- `R-PRM-1`/`R-PRM-3`: add active project and workspace memory, and a small global section, to the input
  list and a rank in the keep order. `R-PRM-1`/`R-PRM-3` global clause: global tooling lessons are the one
  prompt input not bounded by the active workspace. CONCEPTS "Bounded context" likewise; ANA-5 section 4.4
  trim order and the section vocabulary.
- `R-STO-8`: add the memory point type once P5 lands, and replace the draft note that no dedup is
  automatic with the raw-cosine write-time dedup and its Postgres fallback.
- `R-ENT-12` and the owner scopes: a note in ANA-35 (and a one-line owner extension in `R-ENT-12`) that a
  document may be owned at a global scope row, if documents are the home; the scope-row pattern is ANA-35's.
- CONCEPTS lines: Memory (kinds, scopes, states), Lesson, Global scope, and the "Bounded context"
  amendment above.
- A `R-TUI` surface line for the visibility views (list, edit, retire, pin-as-trusted, "why is this here")
  and a Settings cascade line for the capture and promotion switches; the requirement ids to amend were not
  looked up.
- ANA-2 invariant 3 / MOD-98 ("a model never goes inside close-out"): note that the extraction turn is a
  run-end step before close-out and that the formatter itself stays model-free (an inference to confirm).
- ANA-34 section 7.2: the "added to the seed" sentence becomes a concrete reader of active entries; fix the
  stale line references to this item.

## 8. Questions for the maintainer

Plain terms first for each, then the recommendation. Questions 1..10 keep their v1 text; under each is the
maintainer's answer of 2026-10-19. The maintainer answered "all as recommended" and then amended three
points: (a) a new `lesson` kind, (b) a new global scope, (c) memory is completely automated, with no person
acceptance gate. Where an answer says "overridden", the recommendation above it is the superseded v1
position. Question 11 and the residual questions are new in v2.

**1. Is writing a memory entry "doing the work" or "bookkeeping" under R-ID-6?**
In plain terms: step output documents already reach later prompts without anyone approving them, because
they count as the item's own work. A memory entry is different because it lasts beyond the item and
silently shapes every later run. Recommended: treat it as bookkeeping, so an agent can only propose an
entry, and it becomes readable by prompts, the coordinator seed and the index only after a person accepts
it; write that sentence into `R-ID-6` or `R-ENT-13`. Why: a saved entry is a standing instruction, and the
poisoning research and the Augment and Cursor experience say untrusted text must not become one without a
gate. Example: an agent proposes "the sqlx offline cache must be regenerated after any query change"; it
sits in the draft list, the next step's prompt does not include it, and after you accept it at close-out
every later step in the project sees it.

Answer (2026-10-19): accepted, then overridden by complete automation (decision 1c). No person gates
memory: a model writes entries and they become active without approval, so no unaccepted state exists.
The classification is resolved as an explicit exception to `R-ID-6` (amendment text in 7.3, maintainer-gated)
rather than as bookkeeping a person approves. The recommended sentence about unaccepted entries is moot.

**2. Who writes entries, through which channel, and where does a draft live before acceptance?**
In plain terms: you, an agent suggesting something that you approve, or the close-out screen offering you
the item's suggestions in one go. Recommended: three lanes, none new in kind: you write directly; an agent
proposes through an MCP tool into the MOD-135 draft store; the close-out checklist (later coordinator
turns) lists the item's drafts for one confirmation. Close-out code never uses a model to extract
anything. Promoting an existing note (`note_add` exists today) is a useful first step before MOD-135 lands.
Settle also that a draft sits in the draft store, not in the memory table, so no reader can see it. Why:
it is the `R-ID-6` pattern ANA-34 already settled, and `REQUIREMENTS.md:121-131` says drafts are written only
once accepted. Example: during a step the agent calls a propose-memory tool with the subject "raising the
stack in tests", the text and the cited path; at close-out the checklist shows three drafts and you accept
two and edit one.

Answer (2026-10-19): accepted, then overridden by complete automation (decision 1c). The writer is the
configurable light model through the validated deterministic pipeline, at run end and fed by the lesson
detector. The MOD-135 draft store is not required for memory and there is no draft state, so the "where
does a draft live" ruling falls away. A person still may write, edit and retire directly. Close-out code
still never uses a model; the extraction turn is a separate step before it.

**3. Is memory its own table, or a kind of owner-level document from ANA-35?**
In plain terms: either a new kind of row, or a normal document that happens to belong to a project instead
of an item and carries a "memory" label. Recommended: start with an ANA-35 owner document of a conventional
kind; add a dedicated table only if the pin, state and key cannot sit in document columns; do not recreate
ANA-1's `artifact` and `artifact_link` or importance scores. Why: owner documents already give scope,
versions, the "person or accepted draft" rule, the indexer pass and the v4 collection; a new table costs a
Postgres migration, a SQLite mirror migration and new methods in about eight backends, and repeats what was
withdrawn. Risk: a document has no pin or state columns, so the cost may move there. Example: the entry
"tests under htui-orch need the boxed dispatch arms" is a project-owned document of kind `memory` with a
stored pin; it needs no new table and appears in the same project document list.

Answer (2026-10-19): accepted as recommended. Note for P1: states, pin, usage counters, kind and a three-level
scope make a dedicated table more likely than v1 assumed (inference); P1 settles table or document first.

**4. How does an entry stay fresh against the code?**
In plain terms: how htui notices that a remembered fact is no longer true, without asking a model.
Recommended: two checks computed by code. (a) A version stamp on whatever the entry cites, reusing the
existing "citation goes suspect when the requirement changes, a person re-confirms" pattern. (b) For git
projects, an optional pin of repo, commit and cited paths taken from the writing step; a background sweep
(not prompt assembly) flags the entry when those paths changed since the pin. A suspect entry is marked or
held back from prompts until you re-confirm, amend or retire it. A "last confirmed" date is a display aid
only; pin to paths or symbols, never line numbers. Why: stale memory is reported worse than none; the stamp
exists and costs nothing; the pin is deterministic but only works for git projects and cannot run inside the
pure prompt assembler. Example: an entry cites `crates/htui-orch/src/engine.rs`, pinned at one commit; two
weeks later the sweep sees that file changed and flags it "suspect since <commit>"; the next close-out
asks you: still true, amend, or retire.

Answer (2026-10-19): accepted, then adjusted (decision 2). The person step "re-confirm, amend or retire" is
replaced by an automatic re-validation turn, triggered by the sweep and reading the cited paths, which
re-confirms with a new stamp, amends with a new superseding version, or retires; unresolved suspect
entries auto-retire after a configurable period. Freshness is still computed by code and a suspect entry
is still held back from prompts.

**5. How does memory get into a prompt?**
In plain terms: whether a prompt gets a short fixed list of accepted entries, or whatever a search finds.
Recommended: a new internal prompt section filled from Postgres in the engine, accepted non-suspect entries
by scope in a fixed order, a hard token cap, a snapshot at run start so parallel siblings and retries see
the same set, trimmed early (after excerpts, before upstream summaries), unprotected; chosen ids and
versions recorded in the step's trim record; the rest pulled by the agent through a search tool. Amend
`R-PRM-1`, `R-PRM-3` and CONCEPTS "Bounded context" in the same step. Why: the prompt must be a pure
function of its inputs and no model chooses what enters it (ANA-5); a Qdrant ranking would change prompt
bytes when Qdrant is down or re-embedded; Copilot and Claude likewise load a capped recent set or index.
Example: a project has 14 accepted entries and the cap holds 6; the prompt lists 6 with short bodies and
says 8 more are available through search; both fan-out siblings print the same 6.

Answer (2026-10-19): accepted as recommended. The safeguards add deterministic usage counters (injected
into N steps, how many ended done versus failed) for demotion and retirement ranking, and a global section
with its own small cap, ranked below project and workspace; the shape is unchanged.

**6. What is Qdrant's role for memory, and does ANA-38 wait for ANA-37?**
In plain terms: whether memory needs the search index to work at all. Recommended: no. The design works
with Postgres alone; Qdrant is an add-on for agent search and "possible duplicate" hints; memory becomes one
more point type, indexed only once accepted, delivered with ANA-35's planned collection rebuild so there
is one reindex; Qdrant never decides what enters a prompt or whether an entry is fresh; ask ANA-37 to
measure paraphrase recall and duplicate-hint quality on real entries and confirm what ANA-37 decided
before this is finalised. Why: memory is small and scoped, so full-text probably suffices; the only gain
from Qdrant is paraphrase recall and duplicate hints, which ANA-37 can measure. Example: you propose
"tests need --test-threads=1"; the hint says "looks like the existing entry about the process-wide keyring
fake" and you pick merge or keep both; with Qdrant down no hint appears and nothing else changes.

Answer (2026-10-19): accepted, then adjusted (decision 2). Qdrant stays optional and never decides prompt
content or freshness, but its role grows from display-only duplicate hints to improving recall for
write-time dedup through a raw-cosine query; the Postgres-only fallback is the subject key plus full-text.
This makes ANA-37's measurement more valuable, and memory still works with Qdrant absent.

**7. Do ANA-1's atomic facts come back, or do summaries suffice?**
In plain terms: the old design stored many small scored facts linked to each other; the question is whether
to bring that back. Recommended: neither as they were. Keep summaries; add accepted entries with verbatim
prose, a subject, a scope and a freshness stamp; do not bring back the 1-10 importance scores, the fact link
graph or the recursive-CTE injection; if superseding is needed, use one deterministic "supersedes" pointer
you confirm, keyed on (scope, subject). Why: summaries alone do not cover it (the only summary writer is the
mechanical commit table, and summaries reach prompts only through linked items); the evidence says verbatim
text beats extracted facts and that replacing the old entry beats appending. Note this leans on an inference
about what "over-engineered" meant; confirm it. Example: a build-step gotcha from an item unlinked to the
current one surfaces through the project memory section; the older entry on the same subject is marked
superseded, not scored against the new one.

Answer (2026-10-19): accepted as recommended. The `supersedes` pointer is now set by code at write time on
the same (scope, subject) key or high similarity, rather than confirmed by a person.

**8. What is in the first release, and how do we know it pays off?**
In plain terms: how much to build before finding out whether memory helps at all. Recommended: phase P1
only (project entries a person writes or promotes, version-stamp freshness, Postgres-only prompt section, a
TUI list); P2 after MOD-135 and MOD-147; P3 and P4 later; P5, a before/after measurement on real htui steps
and recall of known gotchas, decides whether `R-ENT-13` stays "must" beyond P1. Make it "must" for P1 and
"later" for the rest. Why: P1 needs no draft store, no workspace row and no git read, and it directly
tests the value; the benefit evidence is vendor-reported or abstract-only, so measure on your own data.
Example: in P1 you add "regenerate .sqlx after any query change" to project htui by hand and see it in
every step prompt; agent proposals and the stale flag arrive later.

Answer (2026-10-19): accepted, then adjusted (decision 3). Re-phased to P1..P7 (7.2): "must" for P1 and P2,
"later" for the rest until P7, the measurement phase, reports; MOD-135 no longer blocks memory, MOD-147
blocks workspace and global scope, and MOD-136 is needed only for the coordinator seed.

**9. Where does accepting happen, and how much review is acceptable?**
In plain terms: how many suggestions you are willing to look at, and where. Recommended: reuse the existing
close-out checklist and inbox rather than a new surface; show at most a few drafts per close-out; make
"leave as draft" always safe because drafts have no effect; require typed confirmation only to edit or
retire; agents may propose only from inside a run, with provenance (run, agent, item, commit) stamped by
code. Why: Cursor dropped memories after approval fatigue and unclear scoping; a cap and a safe default keep
the gate from becoming a rubber stamp, and platform-kept provenance is the main defence against
provenance laundering. Example: the checklist shows "Memory candidates (3)" with accept, edit and skip;
skipping all three does not block close-out.

Answer (2026-10-19): accepted, then overridden by complete automation (decision 1c). There is no accept
surface and no review volume to cap, and the close-out "Memory candidates" row is dropped. It is replaced
by human visibility without gating: a TUI list of recent automatic entries with edit, retire,
pin-as-trusted and a "why is this here" view, and per-level switches for capture and global promotion.

**10. What happens to the current development-workflow memory (auto-memory files, Gortex remember,
`.remember`, HANDOFF notes)?**
In plain terms: whether the notes your agents keep today are moved into the new store. Recommended: a
one-time hand-reviewed seed after P1, not a sync; keep agent tooling's own memory as per-user recall
outside htui; htui does not read vendor memory files; record in `R-ENT-13` that product memory is separate
from the development workflow's memory. Why: vendor memories are per-user, per-box and unsynced, and
`R-ID-4` bars writing into repos; several current notes are already stale, so a bulk import would copy
staleness into the new store. Example: the note "code-architect carries `model: opus`" is no longer true;
when you re-add the real gotchas by hand, that one is dropped and each kept entry gets a pin.

Answer (2026-10-19): accepted as recommended.

**11. Lesson kind and global scope (new in v2).**
In plain terms: a gotcha belongs to a project or workspace, but some knowledge is about doing the work
better in general: an error that keeps coming back, a speed-up already found, a habit that makes the tools
behave. Decided by the maintainer: (a) a new kind, `lesson`, for recurring errors, optimizations already
done and things that improve usage over time, not tied to a workflow or an item, owned by a project or a
workspace, with the run as provenance only; (b) a new global scope above workspace, for lessons that can be
generalized and gotchas that can happen in any other project or workspace (tooling and general practice,
not project facts), a third hidden scope row on the pattern of ANA-35's workspace scope row and dependent on
the MOD-147 work; (c) all of it automatic, with the safeguards of Option E-prime in place of an approval
gate. Why: a person does not think to propose the lesson that three runs hit the same timeout, and a gotcha
learned in one project is useful in the next. The price is in section 6: leakage across projects, a bigger
poisoning blast radius, unreviewed standing instructions and a model call on every run. Example: three
runs on different items time out on the same sqlx offline step and then succeed after a cache rebuild; the
detector flags the repeated signature, the extraction turn writes a `lesson` for the project with the error
text and step as evidence, and later the generalization check and validator promote a version stripped of
project names to global, where it appears in every project's prompt under its own small cap.

Answer (2026-10-19): decided as stated above (decisions 1a, 1b, 1c); the design is Option E-prime in 4.6
and the phases are P1..P7 in 7.2. The `R-ID-6` amendment, `R-ENT-13`, `R-PRM-1`/`R-PRM-3`, `R-STO-8` and
CONCEPTS edits are owed (7.3).

**Residual questions (genuinely still open; each with a recommended answer).**

1. **Default sizes.** What are the per-run write cap and the prompt caps per scope? Recommended: configurable
   settings with starting defaults of 5 writes per run, about 1,500 tokens for the project section, 500 for
   workspace and 300 for global (proposals, unmeasured), tuned at P7. The v1 inference of a 1.8k to 7k total
   budget was also unmeasured.
2. **Auto-retire period.** How long may a suspect entry stay unresolved before it auto-retires? Recommended:
   14 days, configurable, applied by the sweep; a retired entry is kept for audit and listed in the TUI.
3. **Global before or after P7 reports.** Phase order puts the global scope (P6) before the measurement
   (P7), while the maintainer wants capture and promotion on by default. Recommended: build P6 after P7's
   first read on P1..P5, so the harm rate of project-level entries is known before one bad entry can reach
   every project; global promotion stays behind its switch either way.
4. **Cost bound for the model.** Is there a ceiling on the light model's token spend for memory? Recommended:
   a setting for a per-run extraction budget, extraction skipped for runs with no observed error, retry or
   measured win, and the per-level off switch; measure the real per-run cost first (section 9.2).
5. **Pin-as-trusted and who may act.** What does a pinned entry escape, and who may edit, retire or pin in
   a shared workspace? Recommended: a pinned entry is exempt from auto-retire and from usage-counter
   demotion but is still held back when suspect; any member allowed to write to the project or workspace may
   edit, retire or pin, with `R-ENT-10` compare-and-set; global entries need the same permission as
   workspace settings.

## 9. Sources and Not verified

### 9.1 Sources

Repo (read by window; Gortex does not index `docs/*.md`, a search for "Dropped" in docs returned 0):
`docs/REQUIREMENTS.md` (R-ID-3/4/6/7, R-ENT-10..15, R-STO-3/8, R-ORCH-11/14, R-PRM-1/3, R-SKL, R-MCP-1/2,
R-NF-2, at the lines cited above); `docs/requirements-review-2026-10-15.md:31-33, 272-276, 290-293`;
`HANDOFF.md:120-135, 418-436, 464-470, 593-604, 748-757`; `docs/ANA-1.md`, `ANA-2.md`, `ANA-5.md`,
`ANA-8.md`, `ANA-9.md`, `ANA-11.md`, `ANA-29.md`, `ANA-30.md`, `ANA-33.md`, `ANA-34.md`, `ANA-35.md`,
`ANA-42.md`; `docs/decisions/ana/ana-1.md`, `ana-11.md`, `ana-13.md`, `ana-18.md`, `ana-30.md`,
`ana-33.md`, `ana-34.md`, `ana-35.md`, `ana-40.md`; `docs/decisions/mod/mod-24.md`, `mod-34.md`, `mod-50.md`;
`docs/htui-mcp.md`, `docs/htui-worker.md`; `CONCEPTS.md`; git commit `f4e24c54` (original `R-ENT-13`
withdrawal text). Code: `crates/htui-core/src/prompt/{mod,trim,template,excerpt,settings}.rs`,
`crates/htui-orch/src/{engine,closeout}.rs`, `crates/htui-store/src/{vector,vector_sync,embed}.rs`,
`crates/htui-store/src/pg/read.rs`, `crates/htui-store/migrations/{0001,0003,0006}*.sql`,
`crates/htui-core/src/model/requirement.rs`, `crates/htui-core/src/store/{traits,mem}.rs`,
`crates/htui/src/{concepts,concepts_worker,worker_cmd,agent_settings}.rs`, `crates/htui-mcp/src/{search.rs,tools/search.rs}`.
Web sources are given inline in section 3. Added in v2: `docs/ANA-35.md:937-947` (the workspace scope row
pattern the global row would copy); the maintainer's answers of 2026-10-19, which are decisions and not
repo evidence.

### 9.2 Not verified

- **Withdrawal rationale.** Only the one-line reason and the commit message exist. That "over-engineered"
  meant the scores and link graph rather than the idea of durable facts is the sweep's inference. The
  recommended "keep entries, drop scores and links" leans on that guess.
- **Docs coverage.** ANA-36 (further MCP tools), ANA-37 (is Qdrant required), ANA-19/20 (Qdrant design,
  any dedup discussion), ANA-3 (providers), ANA-9 beyond the cited lines, the MOD-34/50/68 write-ups
  beyond section-level citations, MOD-38/39 and `docs/htui-mcp.md` line numbers (approximate) were skimmed
  or unread. Other mentions of memory in unopened files may exist. Line numbers marked `~` in the sweeps
  are approximate and given here by section.
- **ANA-37 outcome.** Not on this branch. One reader recalled from an untracked scratch file
  (`.remember/now.md`) that the benchmark ran and the maintainer moved to a "required shared workspace
  Qdrant", with the doc rewrite in progress. That is unverified; if true, the "Postgres-only baseline"
  premise needs revisiting (the design still works, with the index step made mandatory).
- **Code not traced.** Whether current code is on `htui_concepts_v2` or later; whether a v3 name is claimed
  (history said ANA-35 plans v3/v4, substrate and seed cite only v4); whether notes and transcripts are
  indexed in shipped code (`ConceptType` has only Item/Document/Requirement, MOD-112 part 4 open); the
  `concepts_sync_minutes` default (15, from a test name); any Qdrant client request timeout (none found in
  `client()`, qdrant-client defaults unchecked); whether any seeded phase or default template produces a
  `summary`-kind document or places `{{excerpts}}`; the bodies of `PgStore::reconfirm` and the
  `ReqsTab` flow; the full implementor list for a new store-trait method; how a person-gated draft is
  stored (MOD-135 is a filed item with no schema and no `draft` string in any migration; no longer needed
  for memory under v2).
- **Run-record error and retry detail (new in v2).** Whether run records store what the lesson detector
  needs: error text or a signature, retry counts, timeouts, per-run cost. Not verified, and the first task
  of P3. Likewise not traced: how the engine would schedule a run-end extraction turn as a step before
  close-out, and whether the light model setting of `R-ORCH-1` can be called from there.
- **Light-model cost per run (new in v2).** The tokens spent by an extraction turn, a generalization check
  and a re-validation turn are unmeasured, as is the share of runs that would skip extraction; the cost
  lands on the user's plan.
- **Validator recall for project-specific tokens (new in v2).** No measurement exists of how much
  project or workspace-specific text a deterministic validator (names and keys, absolute paths, hostnames,
  secret patterns, private-forge URLs) misses in a real lesson; the global scope's leakage risk rests on it.
  Nor is it verified that an injected string in an error message cannot pass the "evidence must cite the
  run" rule.
- **Vendor CLI memory in htui sessions (new in v2).** Whether a vendor CLI's own memory (Claude auto-memory,
  Codex memories) loads inside sessions htui starts. It matters now because a global lesson injected by
  htui could duplicate or contradict what the CLI already remembers. Also not verified: that the
  ANA-35 workspace scope row pattern carries unchanged to a global row with no workspace, project or repo.
- **Nothing was run.** No Qdrant live test, no build, no embedding of a sample entry, no measurement of
  recall or dedup quality on htui data.
- **Harness prior art** came partly from search summaries and secondary sources: Cursor's memory history,
  Augment's rules details, Gemini's current tool name and approval behaviour, Gortex remember/recall
  internals, Codex's on-disk schema, Copilot's retrieval algorithm. Claude Code's "How CLAUDE.md files load"
  and subagent memory pages were not read in detail.
- **Framework and research numbers** are vendor-reported, second-hand or abstract-only: Copilot merge-rate
  figures, Mem0/Zep/Letta LoCoMo and LongMemEval numbers (LoCoMo is audited as flawed), MemoryAgentBench
  conflict results, TEPA, MERIT, VibeMemBench, DreamBench-SWE, MINJA, MemoryGraft (PDF did not decode),
  Swimm's internals (patented, not public), DOCREF and ADR-tools commands (recalled). Whether Claude's
  auto-memory directory loads under empty setting sources is undocumented.
- **Costs** for commit-pin, hash and symbol-hash checks, the pgvector install burden, and "hundreds to low
  thousands of entries per project" are engineering estimates, not measurements.

### 9.3 Critic findings, kept honest

**Contradictions between the sweeps, and how this draft resolves them**
- Item work or bookkeeping: history says a prompt is a forbidden reader; substrate says step documents
  already reach prompts unaccepted as item work. The classification was never made; the maintainer resolved it in
  question 1 (2026-10-19) as an explicit exception to `R-ID-6`, not as bookkeeping.
- Freshness check timing: the freshness slice says status is checked at injection time; the substrate says
  `assemble` is pure and siblings must be byte-identical. Resolved here by a sweep storing a flag.
- Commit-pin feasibility: the freshness and seed slices treat pinning as cheap and deterministic; the
  substrate finds no branch per run, no hashes for folder projects, rebased pins unreachable, and no
  repo write (`R-ID-4`). The pin is therefore optional and git-only.
- Qdrant status: optional and fail-open per history and substrate; possibly "required" per an untracked
  recollection (above). Unsettled.
- Workspace reach: `R-PRM-1` says inside the active workspace; the engine is headless and project-bounded,
  and a project can sit in several workspaces. Workspace memory waits for the scope row (MOD-147).
- Copilot's 28-day expiry appears in the docs but not in the blog; its check is both "prompted" and the
  model for a deterministic one. Only the shape transfers.
- Dedup: the harness slice says no surveyed system does vector dedup; the framework slice gives
  thresholds (0.98, 0.82) for other setups; the existing RRF score cannot be thresholded and no raw-cosine
  query exists.
- Collection naming (v3 versus v4) is inconsistent across slices; the one-rebuild advice depends on it.
- Close-out extraction: history lists "deterministic extraction from accepted material" as a compliant
  writer, but close-out has no accepted prose to extract. Dropped as a real writer in v1. Superseded in v2: a model extraction turn at run end is now the
  writer, run as a step before close-out so the formatter stays model-free.
- The brief's pointer to ANA-13/ANA-18 for citation re-confirmation was wrong: ANA-13 is the oh-my-pi
  roles study and ANA-18 the Jev model study; the staleness machinery is ANA-11 / MOD-38 / MOD-39.

**Unsupported (claims resting on assumption)**
- That "summaries cover the need" is weak: evidenced structurally (commit-table summary, edge-driven
  reach), not by a measured failed step. ANA-29 is about the maintainer's workflow, not product agents. The
  reopening itself gave no evidence.
- Benefit numbers (see above) support "small and verified", not a specific mechanism.
- Corpus size, token budget (1.8k to 7k) and the claim that Postgres full-text is enough are inferences.
- (v1, moot under v2) Person-acceptance burden is assumed tolerable; no volume estimate exists. Its
  replacement: that the automatic safeguards are enough is assumed, not measured; P7 is the test.
- Skills as zero-new-code memory is asserted from requirement text; cost against `max_skill_tokens` and
  scoping granularity were not checked.
- That MOD-135's draft store can carry a memory proposal: it is a filed item only (moot for memory in v2).
- Parts of Qdrant shipped behaviour (above) and the ANA-37 outcome.

**Uncovered (not decided here, owed before or during filing)**
- Whether memory must be browsable offline (decides a SQLite mirror migration and a `CacheStore` method).
- The TUI surface: list, edit, retire, pin-as-trusted and "why is this here"; reuse of the inbox or a new
  pane.
- Who may edit, retire or pin an entry; multi-user concurrent edits beyond `R-ENT-10`.
- Entry fields, deletion, retention of retired and superseded entries, and the audit trail beyond the
  fragments in section 4.6.
- Precedence between owner levels (project, workspace, global) and between memory and skills or CLAUDE.md
  rules; whether an entry can be promoted from item to project level (global promotion is automatic,
  section 4.6).
- Migration of the maintainer's existing notes (question 10 proposes a hand-reviewed seed).
- Readers beyond a step prompt: chat sessions with no item (only `box_profile` and `search_concepts`),
  judge and handoff roles, a coordinator with no agent.
- A labelling rule for how an entry is phrased in a prompt (data, not directive) and which trust label it
  carries.
- A detailed evaluation plan on htui data (P7 is the placeholder: before/after, recall of known gotchas,
  harm from wrong entries, FTS versus Qdrant).
- Folder projects and remote targets (ANA-42): pins and the repo check do not exist there; only relayed MCP
  and the prompt reach a remote agent.
- Whether ANA-38's product memory is meant to replace the maintainer's own agent-memory workflow or only to
  apply to projects htui manages (question 10 recommends the latter).

**Slices that returned nothing:** none returned empty. Two returned nothing on a specific point: no source
documents an end-to-end human-accept flow for agent memory (searched, not found), and no transcript or
decision file exists for the 2026-09-03 withdrawal.
