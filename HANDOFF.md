# HANDOFF - Outstanding Work (htui)

> **Purpose:** Carry outstanding work between sessions so work resumes.
> `htui` is a Rust TUI that wraps coding agents (`claude`, `agy`) to run a backlog of work items
> through step graphs across projects and boxes. Product contract: `docs/REQUIREMENTS.md`
> (requirement IDs `R-<AREA>-<N>`). Standing invariants: `CONCEPTS.md`.

## How to use this file

- **Session start:** read this file first. Pick the next open item.
- **On completion:** delete the checklist line, write `docs/decisions/<prefix>/<prefix>-N.md`,
  prepend the index line to `DECISIONS.md`, update the summary table (per
  `.claude/rules/workflow-docs.md`; this repo is outside the `sync-workflow-surface` default
  target list, pass `-Targets` explicitly to receive the surface).
- Every item cites the requirement IDs it addresses (`R-NF-4`).

**Current status (2026-09-24):** **MOD-30 was done**
(`docs/decisions/mod/mod-30.md`): the detail sub-tab strip separates titles by one space and a test
pins its width against the pane, so MOD-4 milestone 6 (PRD D5) is unblocked.
Before it, **MOD-35 was done**
(`docs/decisions/mod/mod-35.md`): Added Qdrant connection settings to mirror Postgres DSN configuration.
Before it, **ANA-20 was done**
(`docs/decisions/ana/ana-20.md`): Defined Qdrant feature requirements for MOD-34 (FastEmbed, Hybrid Search, single collection).
**Live coordinates.** Migration `0002_agent_probe.sql` exists, so MOD-4's `0003_orchestration.sql`
is no longer held (`docs/ANA-2.md` §9) and is **still the next migration** — MOD-2 milestone 9 and
MOD-20 both deliberately added none. Adapters install under `HTUI_AGENTS_ROOT`, default
`dirs::data_local_dir()/htui/agents`; `HTUI_TOOL_<NAME>` still overrides everything. Dev Postgres
via `compose.yaml` (port 5439); tests need
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres` and the
`USERNAME=htui-ci` prefix of TOOL-2 (`docs/decisions/mod/mod-6.md`).
**Live coordinates the agent work left, kept here because open items depend on them.** `claude` on
this box is **2.1.267** (2.1.272 at the last estimator re-measure); the seed passes no `--bare`;
`--permission-prompts none` is the deterministic way to provoke a policy denial, and
`~/.claude/settings.json`'s allow list (`Bash(ls *)`) is why the obvious way does not. The CLI
reports `claude_code_version` on `system/init` (there is no `version` key), re-emits `system/init`
on **every turn** of a multi-turn session, and emits a `system/status` row per turn that
`docs/ANA-4.md` §6.2 does not name. This box's live quota blob is `status: "allowed_warning"` at
0.77 utilization and `available()` skips every status that is not exactly `"allowed"`, so the
`claude-cli` row will report `Skip(Status("allowed_warning"))` the moment **MOD-4** has a selection
loop — deliberately MOD-4's to loosen (`quota.rs:403-408`), and a live fact about a real row rather
than a hypothesis. `agy_acp_server` **1.1.1** is installed here and emits **no `usage_update`
whatsoever**, which is why its seed keeps `quota.source: "none"` and why the GPT/Gemini estimator
row cannot be measured on this box (MOD-2 F-17). `--uid=` is **mandatory** for it. Its credentials
live in `$GEMINI_HOME/antigravity-acp/acp_token.json`, a sibling of and separate from the `agy`
CLI's own directory (MOD-21).
**Concluded analyses the open items lean on:** ANA-10 (`docs/decisions/ana/ana-10.md`) — **its
verdict is withdrawn**, see MOD-25 (`docs/decisions/mod/mod-25.md`); the document stays in the tree
as the analysis that was done and not taken, and anything leaning on its local-only half is stale;
ANA-5 (`docs/decisions/ana/ana-5.md`) — the prompt contract, no new crate, no new migration;
ANA-2 (`docs/decisions/ana/ana-2.md`) — step graphs, three compare-and-set status tables,
`htui-orch`. MOD-4, MOD-7, MOD-9, MOD-13 and MOD-14 can start now (MOD-15 is done,
`docs/decisions/mod/mod-15.md`).

---

## Open items

### Analyses


- [ ] **ANA-11 - Models for requirements and decisions.** Evaluate database schema models to track product requirements (R-IDs) and architectural decisions (MOD/ANA items) inside `htui` itself instead of standalone markdown files.
- [ ] **ANA-16 - Research agent execution environments (Docker, remote shell).** Research how to implement ways to run an agent in a Docker container (local and remote) and in a remote shell. Note this would require a central server with htui as just the interface.
- [ ] **ANA-17 - Per-model calibration of the prompt's section framing** (from MOD-2, finding F-37).
  `R-PRM-1`, `R-PRM-2`. ANA-5 fixes the `<section name="...">` wrapper but never fixes what separates
  the N blocks a single placeholder expands to — `{{documents}}` renders one block per document,
  `{{candidates}}` one per candidate. MOD-2 shipped a blank line between them
  (`prompt/render.rs`, stable under §4.7 step 5's "collapse 3+ LFs to 2"), chosen for readability
  rather than measured against any model. The question this analysis owns is whether the framing
  should be **calibrated per model at all** — separator, and plausibly the wrapper itself — since
  the registry already carries a row per agent and `TokenEstimator::for_agent` already varies by
  model family (D108). Any verdict has to price the cost of varying it: the separator bytes are
  digest input, so a per-model frame means `prompt_digest` is only comparable within one model, and
  every golden snapshot in `crates/htui-core/tests/snapshots/` is re-recorded on each change. Leave
  the current blank line in place until this concludes.
- [ ] **ANA-21 - Per-model weights for agent assignment, derived from public sources** (from MOD-4
  milestone 4, OQ-7; maintainer-requested 2026-09-23). `R-AGT-8`, `R-ORCH-7`. Blocks MOD-36.
  Research how to give each configured agent/model a weight (e.g. Gemini Flash 30, Claude Opus 5.5
  60) that MOD-36 uses to choose which models run a phase's fan-out candidates. Survey public signals
  (coding and reasoning benchmarks, leaderboards, published pricing and latency), decide whether
  weights are per task kind (analysis, implement, review, judge) or global, and whether cost enters
  the weight or stays a separate quota concern (ANA-4). Deliver: the weight table's schema and where
  it lives (`agent` row, `app_setting`, or a new table), a refresh method (manual, scripted from
  named sources, or learned from htui's own judge verdicts), and initial values for the seeded agents.

### Next features
- [ ] **MOD-36 - Weighted agent assignment across fan-out candidates** (from MOD-4 milestone 4,
  OQ-7; blocked on ANA-21). `R-AGT-8`, `R-ORCH-7`. Milestone 4 runs every candidate of a group on the
  one agent the walk selects (rival sampling). This item spreads candidates across the eligible
  agents by weight: a weighted `AgentSelector` (the seam milestone 4 leaves per-candidate) picks each
  `fanout_index`'s agent from the walk's eligible list using ANA-21's weights, so the judge compares
  different models on the same task. Open points to settle in the plan: the prompt is assembled once
  per group, but token budgeting uses one agent's estimator (`TokenEstimator::for_agent`), so mixed
  families need either the tightest budget or per-candidate trimming, which breaks "identical prompt
  across siblings" (ANA-5 `:1870`); each candidate draws on its own agent's quota; the judge must not
  learn which model wrote which candidate (position-bias control extends to model-identity bias).
- [ ] **MOD-34 - Qdrant related concepts search** (from ANA-19). Update `compose.yaml` to include the `qdrant/qdrant` image. Introduce `VectorStore` trait and `QdrantStore`. Implement local embedding generation using `fastembed-rs` (avoiding external APIs per ANA-20). Implement hybrid search (Dense + BM25) and single-collection payload indexing for `docs/` and items, then wire semantic search to the `search_concepts` MCP tool.
- [ ] **MOD-33 - The box hostname leaves the digest and gains a settings switch** (from MOD-2,
  finding L-5; maintainer-decided 2026-09-16). `R-PRM-1`, `R-PRM-3`, `R-TUI-8`. Two changes to the
  box section (`prompt/render.rs`, the §4.2 projection over `BoxProfile`):
  **(1) the hostname stops being a digest input.** Today an identical `PromptSpec` assembled on two
  boxes produces different bytes and therefore a different `prompt_digest`, so a digest can only be
  compared within one machine — which defeats MOD-4's criterion 11 (comparing a real step's digest
  against the preview's) the moment the two run on different boxes. The hostname stays *in the text
  the model sees*; it is excluded from the bytes that are hashed. That split does not exist yet:
  §4.7's pipeline hashes exactly what it renders, so this needs a "rendered but not digested" span
  concept, and the `trim_record` must say the prompt carried one.
  **(2) a setting decides whether it is rendered at all.** The maintainer's case for keeping it:
  building a graphics engine across several machines, where the agent must know which box it is
  building on. The case against: it is a machine identifier in text sent to a model. So it becomes a
  per-project switch (`app_setting`/`project.settings`, defaulting to on to preserve today's
  behaviour), surfaced in Settings, with the box section omitting the field entirely when off.
  **This amends ANA-5 §4.2** (the closed box field list gains a conditional field) **and §4.7**
  (rule 8's "no machine identifier" becomes true of the digest rather than of the prompt) — the
  amendment is maintainer-approved and recorded here per the milestone-5 precedent that an ANA edit
  is maintainer-only. Its write-up should carry the ANA-5 sections it touches.
- [ ] **MOD-31 - A running prompt preview makes an adapter install refuse** (from MOD-2, finding
  F-121). `R-AGT-10`, `R-TUI-8`, `R-NF-3`. `AgentRuntime::serve` pushes the deferred preview task
  into `self.background` (`agent_worker.rs:824`), and the install guard refuses whenever
  `!self.background.is_empty()` with *"a probe is already running on this box; install once it has
  finished"* (`agent_worker.rs:1116`). Selecting a Backlog row therefore blocks `i` in Settings for
  the life of a preview, with a message about a probe that is not running. The guard's reason is real
  — a probe and an install's re-probe race on the same `agent_box` row — but it keys on the wrong
  set: `background` mixes tasks that **write** `agent_box` (the probe, the re-probe) with tasks that
  only **read** (the preview, plan D102's "the preview writes nothing"). Split `background` by what a
  task writes, and let the install guard consult the writing half only. `background_len` is read by
  tests, so the split has to keep an answer for them. Found at MOD-2 close-out, 2026-09-15.
- [ ] **MOD-32 - `trim_record`'s own strings reach the store unscrubbed** (from MOD-2, finding
  F-80). `R-SEC-3`, `R-PRM-3`. MOD-2's assembler scrubs every **digested** byte at the input layer
  (D100 as corrected by the milestone-9 CRITICAL, `f48b82b`), so nothing unmasked reaches the model
  or `prompt_digest`. The record written beside it does not get the same pass: `trim_record.notes`,
  the `excerpts` audit's paths and root strings, and `budget_source`/`estimator` text are generated
  strings serialised straight into `run_step.trim_record` by `set_step_prompt`. The exposure is small
  by construction — the strings are repo-relative paths and enum spellings — but `R-SEC-3` gates the
  **persist** path rather than the prompt path, and a fail-closed scrubber that is not called is not
  fail-closed. Decide between scrubbing `TrimRecord::to_value()`'s output before the write and
  refusing the write on residue, the way the assembler refuses. **Not MOD-10's**: MOD-10 replaces the
  `Scrubber` implementation behind an unchanged trait; this is a missing call site. Found at MOD-2
  close-out, 2026-09-15.

- [ ] **MOD-28 - rataflow execution view (from ANA-12).** Add `rataflow` dependency, implement `ExecutionGraph` widget mapping `RunStep` and `SessionEvent` lists to a node graph, add view toggle to Runs tab (`R-TUI-4`), and wire mouse/keyboard events for standard run actions.
- [ ] **MOD-26 - Declarative Agent Personas (from ANA-13).** Build Markdown/Frontmatter parser in `htui-core`, discover from `~/.config/htui/agents.d/`, map to `SessionSpec` overrides (model, tools).
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees.
- [ ] **MOD-4 - Orchestrator, manual mode** (from ANA-2). `R-ORCH-1..5`, `R-ORCH-7..11`,
  `R-TUI-4`, `R-TUI-9`. Step graphs per kind, gates, retries, review loop, fan-out with isolation
  modes and selection, capability check, promotion to chat, run records, Runs tab actions, and
  close-out (summary document, status, commit hashes). The `run` and `close` actions of `R-TUI-2`.
  Design concluded in `docs/ANA-2.md` (ANA-2, `docs/decisions/ana/ana-2.md`): new crate
  `htui-orch` plus `crates/htui/src/run_worker.rs` (§8), migration `0003_orchestration.sql` and
  `cache_migrations/0003_orchestration.sql` (§9; never applied before MOD-2's `0002`, **which
  landed 2026-09-08 in `fb626a8`, so this constraint is now satisfied**. **The cache
  migration is `0003`, not the `0002` ANA-2 §9 reserved:** MOD-2 milestone 4 spent
  `cache_migrations/0002_agent_mirror.sql` mirroring the registry so an offline chat can start), plus
  **`local_migrations/0002_orchestration_local.sql` in the same commit** (ANA-10 §5.3's Set B — a
  `0003` landing alone leaves the local schema a generation behind, and SQLite makes catching up
  expensive), **eighteen** `WriteStore` methods (ANA-2 §8's table is 17 rows and `docs/ANA-2.md:1744`
  names two; §1760's "sixteen" undercounts its own table) plus the six `ReadStore` reads of
  `:1730-1733` and the fifteen new inherent reads of `:1711-1724`, each now obliging **three** stores
  — `MemStore`, `PgStore` and ANA-10's `LocalStore` — with conformance cases, `can_move_to` on the
  three status enums (§4.3), typed `BoxSettings`/`ProjectSettings` (§4.7), projection additions
  (§6.2), fixture corrections (`attempt` 1-based, non-NULL `graph_snapshot`, `review` in
  `implement.input_kinds`, `gate_hard` seed), `gix` as the git dependency; build order in §9,
  steps 1 to 4 need nothing from MOD-2. Per ANA-5 (`docs/ANA-5.md` §4.6, §8): calls `assemble()`
  for the judge and handoff prompts, supplies `verify_failure`/`previous_diff` for the review loop,
  `RunStepSummary` gains `prompt_tokens` and `trimmed`; the pre-flight digest write is MOD-2's
  `set_step_prompt`, not `finish_step`. **ANA-10's M8 is struck from this item** (MOD-25 closed
  2026-09-16, `docs/decisions/mod/mod-25.md`): `htui` is online-only, so there is no `LocalStore`,
  no local-only graph parity, no four SQL ports and **no
  `local_migrations/0002_orchestration_local.sql`** — the migration set is `0003_orchestration.sql`
  plus `cache_migrations/0003_orchestration.sql` and nothing else. Two stores oblige the seam,
  `MemStore` and `PgStore`, not three. The `PgStore`-inherent reads of ANA-2 §8 become
  `Backend`-inherent with a `match self` (`agents()` precedent, `backend.rs:292-298`); §8's split
  rule itself is unchanged. One prohibition survives: never `CHECK (fanout_index >= 0)` on either
  schema (ANA-2 risk 12); ANA-10 §10.17's local `shared_serialized` question is moot.
  **Not blocked** — MOD-2 is done (`docs/decisions/mod/mod-2.md`), MOD-6 landed
  (`docs/decisions/mod/mod-6.md`), and MOD-25 removed the MOD-17 M3 ordering rule by withdrawing
  MOD-17 (not by satisfying it). **Three things MOD-2 handed over by name.** (1) ANA-5 §12
  **criterion 18's persistence half** is this item's (MOD-2 D107): a handoff prompt must be
  persisted as a `follow_up` event at the next `turn`, not as a second `prompt` row, leaving
  `run_step.prompt_digest` unchanged — it needs a promotion, which is `R-ORCH-5`. The assembler half
  is proved by `prompt_digest.rs::a_handoff_summary_carries_only_the_windowed_tail`. (2) ANA-5 §12
  **criterion 3's `blocked` transition** and **criterion 16's front-matter parser** are likewise
  MOD-2-adjacent and this item's. (3) MOD-2 finding **F-104**: `excerpt::select`'s `vetted()` refuses
  any provider candidate the reader's listing never offered, and a listing truncated by `scan_cap` is
  only a prefix — so on a very large repository a legitimate provider candidate is refused and noted.
  That is the conservative direction and the price of not leaning on `htui-agent` for a security
  property; loosening it (a per-candidate stat under the same symlink discipline) is this item's
  call, not the assembler's. Also inherited: `available()` skips every quota status that is not
  exactly `"allowed"`, so this box's `claude-cli` row reports `Skip(Status("allowed_warning"))` the
  moment there is a selection loop (`quota.rs:403-408`).
  **Milestone 1 landed (`33277b1`..`e3163ca`, 2026-09-18): the seam knows what a run is.** 37
  commits, 120 files; workspace green at `--test-threads=1` (1304 passed), `sqlx prepare --check`
  clean. `WriteStore` 42 → 60, `ReadStore` +5 (including ANA-2 §4.2's `resolve_inputs`, which
  `documents_of_kinds` deliberately is not), 11 `Backend`-inherent reads, the three §4.3 tables as
  `can_move_to` with the refusal in the seam, `MemStore` + `PgStore` over
  `0003_orchestration.sql` and its `cache_migrations/0003` companion, `run_step_tree` as the
  seventeenth mirrored table, `CASES` 36 → 47 and `READ_CASES` 6 → 9. **Six milestones, not the
  seven PRD D1 approved**: the traits carry no default bodies, so PRD milestones 1 and 2 cannot
  each close on a green workspace and are one milestone (plan D0); ANA-2 §9's build order is
  unchanged. **Two fixture corrections of this item's list are now applied** (`attempt` 1-based, a
  real `graph_snapshot` on both graph runs); the other two were already MOD-15's. **Two seam
  questions the review gate raised are deferred to milestone 2**, recorded in
  `.claude/plans/mod-4-orch-seam.blueprint.md` §4.6 R-1/R-2: a terminal `transition_run`/`fail_run`
  does not write the item mirror in the same transaction, which ANA-2 §4.3:543 asks for and ANA-2
  §8's writer table does not provide; and `claim_run`'s overlap predicate is repo-set-on-one-box,
  so §4.7's isolation-aware rules I and P cannot be evaluated inside the admission critical
  section — two worktree-isolated runs on one repo are refused, which is safe but narrower than
  `R-ORCH-9`. Plan: `.claude/plans/mod-4-orch-seam.plan.md`; blueprint:
  `.claude/plans/mod-4-orch-seam.blueprint.md`.
  **Milestone 2 landed (`cacab31`..`0a53d9c`, 2026-09-19; close-out round `f87c2b7`..`1cbddc5`,
  2026-09-22): a graph walks.** 22 implementation commits over 37 files, plus eight close-out
  commits. `crates/htui-orch` is the fifth workspace crate — `lib.rs`, `graph.rs`, `status.rs`,
  `command.rs`, `isolate.rs`, `engine.rs`, `gate.rs`, and `fake.rs` + `conformance.rs` behind
  `test-support` — depending on `htui-core` and `htui-agent` and never on `htui-store` (ANA-2
  invariant 10). It runs `prd -> plan -> implement -> review` against `FakeDriver`, a new
  `FakeIsolator` and `MemStore`: the six-stage walk, the gate table, the review loop and its
  no-progress predicate, with no git, no Postgres, no agent and no second migration.
  **Milestone 1's deferred R-1 is answered**: `finish_run` is the nineteenth writer and moves the
  run and mirrors its item in one transaction on both stores (`PgStore` takes `run` then `item`
  `FOR UPDATE`, the order every other two-row writer already uses). **R-2 is still deferred to
  milestone 5** — the engine adds no overlap reasoning of its own. `CASES` 47 -> 48; `htui-orch`'s
  own case list is 15.
  **Three decisions the implementation corrected, all recorded in the plan**: D5 retires a `failed`
  step by cancelling it rather than leaving it alone (a `failed` latest attempt reads as a rest, so
  the walk would stop forever); D10 records an unexpected review verdict as an `item_note` carrying
  `via_step_id`, not in `gate_note`; D19 resolves the graph through a `GraphSource` trait
  `htui-orch` defines itself, because eleven of milestone 1's reads are store-inherent and not on
  any trait.
  **The review gate found three HIGH and five MEDIUM; every finding was verified against the tree
  and every one was applied** (`9acb166`, `106671b`, `23fef33`, `9870673`, `60120c7`, `6345a4a`,
  `ed7756c`, `c83ae97`, `cf2f08a`, `5d4582a`, `1cbddc5`). The two that change behaviour a later
  milestone must know about: (1) a step that errors *after* the `pending -> running` move used to
  leave the run running forever, because `cursor` rests on `running` and every command refuses a
  running step — the walk now delegates to `walk_live_step` and hard-fails through `fail_hard`
  (`running -> failed` + `finish_run`), and **a spawn failure lands in `failed` under every gate**,
  never parking, per ANA-2 `:639`; (2) `RetryStep` on a terminal run created an orphan `pending`
  step and reported success — neither store refuses a step on a terminal run, so the refusal is the
  engine's (`EngineError::RunStatus`). ANA-2's `failed -> retry -> queued` row (`:585`) is the
  *item* table and remains unimplemented. `EngineError` also gained `Snapshot` and `Stalled` so the
  walk's own invariants stop arriving as `Store(Constraint)` — milestone 6's `run_worker.rs` is the
  first consumer that would have been misled.
  **A second §4.2/§4.3 disagreement is now recorded in the plan's risks, beside D2's**: ANA-2
  `:450` says a `never` gate with an exhausted `failed` settle ends the run `awaiting_approval`,
  while `:575`/`:608` and the shipped code end it `failed`. Milestone 5 must not re-read `:450`
  literally. Two known warts, both deliberate: an engine-side encode failure of the trim record is
  reported as `EngineError::Record(RecordError::Encode)`, whose sentence says "recorder"; and
  `Rest.failure` is populated only by the transition that caused the stop, with `run.failure` as
  the durable record (documented rather than parsed back, so D12's one-way grammar keeps one home).
  Plan: `.claude/plans/mod-4-orch-engine.plan.md`; blueprint:
  `.claude/plans/mod-4-orch-engine.blueprint.md`.
  **Milestone 3 landed (`6307a3d`..`1b78753`, 2026-09-22): work happens in a real tree.** 22
  implementation commits plus a seven-commit review round, 39 files; workspace green at
  `--test-threads=1` (1445 passed), `sqlx prepare --check` clean. `htui-orch` gains
  `isolate/git.rs`, `isolate/real.rs` (`GixIsolator`, all four `R-ORCH-8` modes), `isolate/copy.rs`
  and `verify.rs` (`verify_command` through the platform shell, three outcomes, deadline, tail cap,
  scrubbed output); the engine calls the verify hook between stages 4 and 5, reconciles the winner
  into the primary tree and cleans up, and `Command` gains `CancelRun`. The seam gains
  `record_command_run`/`command_runs` and `upsert_step_tree` now also writes
  `run_step.isolation_path`; `CASES` 48 -> 49. ANA-2 criteria 11, 12 and 13 are proved over a real
  repository (`tests/gix_isolator.rs`, `pg_criteria.rs`).
  **OQ-1 was resolved to the `git` CLI, and that has a cost.** `gix` 0.87.1 ships no worktree
  mutation, so `isolate/git.rs` shells out for exactly five verbs — `worktree add --lock`,
  `worktree remove`, `merge --no-ff`, `merge --abort` and `reset --hard` (D47) — with a floor of
  git 2.33.0, checked once when the isolator is built; `gix` keeps every read and every ref write,
  and its sync calls run on the blocking pool. `docs/ANA-2.md:1777` and `:2055` are amended. **`git`
  is therefore a runtime dependency of the `worktree` mode and of reconciliation**, and one more
  fact for MOD-16's Windows verification. **`git worktree prune` is never run** (D46): a pruned
  admin entry is instead detected when a tree's HEAD cannot be read, and the tree is re-made onto
  its existing branch.
  **The review gate found three HIGH, four MEDIUM and five LOW; every finding was verified against
  the tree and every one was applied** (`892fcf3`, `ff47431`, `085fb79`, `2392031`, `a7e929f`,
  `70bc276`, `1b78753`). The behaviours a later milestone must know: (1) **a no-commit `worktree`
  tree is kept, not removed, when it holds untracked work** — `is_dirty` deliberately ignores
  untracked files (D24, which the dirty-tree refusals rely on), so the step-end removal also asks
  `has_untracked_files`; (2) **the `shared_serialized` guard lives from `prepare` to that step's
  capture, and `cleanup(run, _)` drops every guard of the run whether or not it has `step_trees`
  rows** — a failure between `prepare` and `upsert_step_tree` used to leak the guard until restart;
  guards are taken in `RepoId` order so milestone 4's siblings cannot deadlock; (3) a failed merge
  always runs `merge --abort`, even when its conflicted paths cannot be read; (4) a copy never
  carries a `*.lock` out of the source's `.git`; (5) lock retries fire only on the exact lock
  wordings, never on a message that merely names `Cargo.lock` or a ref-name conflict.
  **Still carried**: R-3..R-6 from milestone 2; **R-7** — a run parked by a reconcile refusal
  (`park_run`) has no resume verb, so milestone 5's sweep or milestone 6's `Unblock`-shaped verb
  must provide one; **R-8** — `after_hash` becomes the merge commit after reconcile (H-19), so
  milestone 4's no-progress predicate may need the pre-merge hash. Plan:
  `.claude/plans/mod-4-orch-tree.plan.md`; blueprint: `.claude/plans/mod-4-orch-tree.blueprint.md`.
  **Milestone 4 landed (`fa03782`..`7ee0ac6`, 2026-09-23): three candidates, one winner.** 36
  implementation commits plus a thirteen-commit review round, 29 files; workspace green at
  `--test-threads=1` (1554 passed, 0 failed, 26 ignored), `sqlx prepare --check` clean. `htui-orch` gains `select.rs`
  (`R-AGT-8`'s candidate walk, D60) and `fanout.rs` (the pure half of selection); the engine drives a
  fan-out group concurrently, prefilters on `verify_command`, runs the two-order judge as one step
  with two sessions, and selects one winner in one transaction, with `Command::SelectFanout` for the
  human path and `RetryStep` retrying a parked group as a whole. The `Isolator` seam widens for
  fan-out (a slot on `prepare`, `base`, `diff`, a sibling list on `reconcile`); `git diff` is the
  sixth CLI verb, and `worktree add`/`remove` are serialised per repository (D70) because concurrent
  adds race on `.git/worktrees/<id>/commondir`. `htui-core`'s quota predicate treats
  `allowed_warning` as selectable (D61). ANA-2 criteria 7 (real-tree half, via R-8), 8, 9 and 10
  are proved; `htui-orch`'s case list is 36. **R-8 is closed with no predicate change** (D66).
  **The review gate found one HIGH, three MEDIUM and six LOW; every finding was verified against the
  tree and every one was applied** (`20cb2fa`, `8438406`, `86058a9`, `a40b831`, `ad60554`,
  `375123b`, `87ec1d4`, plus doc repairs `a19e633`, `ffe86c0`, `1fa37e2`, `7ee0ac6`). The
  behaviours a later milestone must know: (1) **a candidate's deadline and verify budget start when
  its `prepare` answers**, not at `run_step.started_at` — in a `shared_serialized` group the later
  siblings were being charged for the earlier ones' whole sessions; (2) **a retried group starts
  from the base its retired attempt recorded** (`before_hash`), not from HEAD, which in
  `shared_serialized` is the last loser's commit — a retired slot that names two bases, or none for
  a repo in scope, is refused with `EngineError::GroupBase`; an attempt with no rows at all is
  skipped, and a slot with a selected winner (the review loop) still starts from `Isolator::base`;
  (3) a store error after the judge went `running` now fails the judge and parks the run instead of
  leaving both `running`; (4) **`resume` keeps walking the snapshot when the live graph no longer
  resolves** under lowered caps (`FanOutCap`, `AgentCap`, `ReviewFanOut`, `NoCandidate`) and writes a
  note — invariant 2; (5) **`drive_group` is not cancel-safe**: dropping it leaves candidates
  `running` and shared locks held until `cleanup_run`/`cancel_run` or milestone 5's sweep; (6) a
  stale-member `RetryStep` is `EngineError::StaleSlot`.
  **The plan's "no migration" held until the review gate, then deliberately did not**: the
  maintainer raised `max_agents_per_run`'s default from 6 to 8 (OQ-1 revisited), because the literal
  reading refused the seeded `feature` graph with a judged 3-way `implement` (7 agents).
  `0004_max_agents_per_run_default.sql` moves an untouched seeded `6` to `8` and leaves any other
  value alone (`0003` is on `main` and cannot be edited); `graph.rs`'s fallback follows (`b86b62c`,
  `bcce4b9`). A judged
  4-way `implement` (8) sits exactly at the cap. **`docs/ANA-2.md:871`, `:1472`, `:1516` and
  `:1992` and `docs/decisions/ana/ana-2.md:60`, `:107` still say 6** and are not amended.
  **Still carried**: R-3..R-7 from milestones 2 and 3; **R-9** — `LoopStop::NoProgressReview` has
  been unreachable since milestone 2, because `reviews_are_identical` reads the latest-only
  `documents_of_kinds` (blueprint F-B; the plan's risk row was corrected in `e2986f2`), so only the
  `after_hash` half of the no-progress predicate can fire. Every production judge still fails until
  MOD-11 gives an agent a way to write the `judge` document (OQ-4), so production fan-outs go to the
  human. Plan: `.claude/plans/mod-4-orch-fanout.plan.md`; blueprint:
  `.claude/plans/mod-4-orch-fanout.blueprint.md`.
  **Milestone 4 OQ-7 follow-up: MOD-36** — milestone 4 runs every fan-out candidate on one agent;
  spreading candidates across agents by weight is MOD-36's (weights from ANA-21). Milestone 4 leaves
  `AgentSelector::select` called once per candidate with its `fanout_index` so MOD-36 plugs in there.
  **Milestone 5 landed (`a1fb291`..`8dc4755`, 2026-09-23): two runs do not collide, and a crash is
  survivable.** Eight tasks (T1–T8), then two review rounds. Workspace green at
  `--test-threads=1` (1700 passed, 0 failed, 26 ignored), `sqlx prepare --check` clean (227 files),
  store conformance 53 cases, `htui-orch` case list 52. `htui-core` gains `model/overlap.rs` (the
  scope predicate, D80); `htui-orch` gains `overlap.rs` (scope resolution at `StartRun`) and
  `recover.rs` (the heartbeat and the sweep's adjudication). `claim_run` answers a `Claim`, and new
  store verbs are `take_lease`, `interrupt_step` and `release_lease`. Every walk runs under a
  leased heartbeat that fences itself before the lease lapses (D122, D143); every compare-and-set on
  a walk path honours `Ok(false)` as `EngineError::StaleWrite` (D125, D144, exemptions named on
  `Engine::move_step`); the sweep adopts expired runs one at a time, and a process takes back its
  own dead walks through an in-process `DeadWalks` set (D139, D140). `reconcile_isolated` merges
  onto a primary another run moved, under the repo's admin lock (D136). No migration.
  **Round 1 (D122–D138) fixed four HIGH, four MEDIUM and several LOW; round 2 (D139–D145) fixed one
  HIGH, two MEDIUM and three LOW** — blueprint §21–§22. The final reviewer approved with fixes; two
  LOWs were applied (`ced0808`, `8dc4755`) and three MEDIUMs are carried. **Carried to milestone 6**:
  R-26..R-35 (blueprint §21.2, §22.3), chiefly R-27 (no per-run mutex between commands in one
  process), R-33 (D141 recognises htui's reconcile merge by its message only, spoofable by an agent
  and missed under `merge.log=true`), R-34 (a command failing between `take_lease` and
  `walk_leased` keeps its live lease, so its own sweep skips the run until restart) and R-25 (a
  failed terminal cleanup is never retried). About six intermediate commits fail `clippy -D
  warnings` on their own; HEAD is clean. Plan: `.claude/plans/mod-4-orch-lease.plan.md`;
  blueprint: `.claude/plans/mod-4-orch-lease.blueprint.md`.
  **Milestone 5 follow-up landed (`0ea3744`..`1d598f1`, branch `mod-4-m5-fixups`, 2026-09-24): R-33,
  R-34 and R-35 closed** (blueprint §23, D146–D152). `git::reconcile_parent` accepts a reconcile
  merge only on the primary's first-parent line, for `worktree`/`copy` rows, by subject, and
  `merge_no_ff` runs with `merge.log=false`; every command's window between its lease take and its
  walk gives the lease back on error (`Engine::leased_window`), as does a topology-mismatch resume;
  a dead walk whose run is gone leaves `DeadWalks`, told apart from an outage by a `test-support`
  `MemFault` hook on `MemStore`. Workspace green at `--test-threads=1`, pins unchanged (store 53,
  orch 52, `.sqlx` 227). Final review approve-with-fixes, all applied but L7, carried as R-37 with
  R-36 (a mismatched `running` run is re-adopted each sweep). `0e61f08` alone fails clippy.
- [ ] **MOD-7 - Box registry + capabilities.** `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6`, `R-TUI-8`.
  Probe, registration, capability tags and quirks editor (Settings tab box profile section),
  per-box paths, agent autodiscovery hook. Not blocked (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`: `register_box` writes the minimal row, the probe fills the rest).
  MOD-4 and MOD-12 need `probed_tags`/`declared_tags` from a real probe, `repo_box_path` rows for
  every isolation mode, and `agent_box.probe.status` (`docs/ANA-2.md` §4.10, §7). `repo_box_path`
  rows and a real probe turn ANA-5's excerpt fallback root and box profile projection
  (`docs/ANA-5.md` §4.2, §4.5) from degraded into complete. **MOD-20 landed**
  (`docs/decisions/mod/mod-20.md`): `docs/ANA-4.md` §4.6 named this item as the possible owner of
  registry-driven installation, and it is now built and `R-AGT-10`-backed - so this item *calls*
  `htui_agent::install` from its box registration and probe hook rather than growing one, and that
  hook is the natural second caller of the action `Settings > i` already exposes (MOD-20 D7 kept
  the MVP to Settings deliberately). **MOD-2 shipped the consumer and left one gap here by name**
  (finding F-102): `htui_agent::excerpt::FsRepoReader` refuses a symlink at any component *below*
  the root, but a `RepoRoot` whose **own** path is a link is resolved by whoever writes
  `repo_box_path` — which is this item — and nothing writes it yet. Whatever writes those rows must
  canonicalise or refuse a root that is a link, or the excerpt walk's symlink discipline starts one
  component too late.
- [ ] **MOD-9 - Skill library and templates.** `R-SKL-1..4`, `R-PRM-4`, `R-TUI-7`. Versioned skills,
  project and phase bindings, template rows, Skills tab editor with version diff, import of
  existing skill markdown files. Per ANA-5 (`docs/ANA-5.md` §4.1, §5.4): template save validation
  calls `htui_core::prompt::template::parse`; `judge` and `handoff` are reserved names whose
  `TemplateRole` derives from the row name; the closed placeholder tables are the editor's inline
  help. Not blocked, and the dependency is now satisfied: **MOD-2 shipped the validator**
  (`docs/decisions/mod/mod-2.md`), so ANA-5 risk 11 is closed. Three things are waiting here by
  name. `htui_core::prompt::template::parse` refuses with a **byte offset**, so the editor can put
  the cursor on the mistake rather than reporting "invalid". `htui_core::prompt::render::template_text`
  is deliberately kept though the assembler no longer calls it (MOD-2 finding F-83, `fd9e752`): it is
  for exactly this editor, which has a `ParsedTemplate` and no scrubber, and its doc says so. And the
  skill model layer is here already, read-only (MOD-2 D105) — `Skill`, `SkillVersion`,
  `SkillBinding`, `BoundSkill` with the `R-SKL-2` collapse and the `max_skill_tokens` cap — so this
  item owns only the writers `upsert_skill`, `add_skill_version` and `set_skill_binding`, plus the
  editor. `htui_core::prompt::defaults::DEFAULT_TEMPLATES` is the ten bodies to seed *from*; seeding
  them into a project's `prompt_template` rows **landed with MOD-15** (ANA-5 §4.6,
  `docs/decisions/mod/mod-15.md`): `htui_core::seed` writes all ten at version 1 on create.
- [ ] **MOD-10 - Secret provider** (from ANA-7). `R-SEC-1..4`, `R-TUI-8`. `SecretProvider` trait,
  Infisical implementation, environment injection at run start, scrubber with exact-match and
  pattern masks, fail-closed persistence gate, Settings tab secret provider section. **No longer
  blocked** — MOD-2 is done (`docs/decisions/mod/mod-2.md`) and shipped the `Scrubber` seam with the
  fail-closed `MinimalScrubber` this item replaces *behind an unchanged trait*. Its call sites are
  already fail-closed on every digested byte (MOD-2 D100 as corrected by that milestone's CRITICAL);
  the one **missing** call site is **MOD-32**, not this item.
- [ ] **MOD-11 - htui MCP server.** `R-MCP-1..4`. Tools `item_link`, `item_status`,
  `document_write`, `note_add`, `box_profile`, `command_run`; per-step scoping; command queue with
  per-box class limits; per-phase exposure. Per ANA-2 (`docs/ANA-2.md` §4.2, §8, risk 11):
  `document_write` calls `WriteStore::write_document` (orchestrator-allocated version), `command_run`
  accepts class `verify` for `verify_command`, and an `item_status` request is recorded as an
  `item_note` with `via_step_id`, never a transition. The `box_profile` read tool returns ANA-5's
  box profile projection (`docs/ANA-5.md` §4.2) so the tool and the prompt section agree — MOD-2
  shipped that projection as `htui_core::prompt::BoxProfile::project`, which drops `box_tool.path`
  (ANA-5 §4.2 rule 5), so the tool must not re-add it. **Blocked on MOD-4 only** now that MOD-2 is
  done (`docs/decisions/mod/mod-2.md`); MOD-2's own `permission_request` gap over the CLI transport
  stays declared until this item lands, since the `--permission-prompt-tool` contract is its route.
- [ ] **MOD-12 - Auto mode queue runner** (from ANA-2). `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard
  gates, `R-AGT-7..8` caps, `R-TUI-8`. Ready-item selection, capability filter, concurrency with
  overlap rule, queue overlay, escalation, Settings tab caps and scheduler window section. The
  `queue` action of `R-TUI-2`. Target box stored, local execution only. Design concluded in
  `docs/ANA-2.md` (§4.10, §9): `ready_items` per ANA-9 §7.4 in full, the same `claim_run`
  admission as MOD-4, batch caps as `SUM(run_step.usage)`, escalations in the queue overlay, the
  `scheduler_window` key stored but not enforced; a graph with a `gate_hard` phase is never fully
  unattended, so `FIX`/`CLEAN`/`TOOL` are the first targets. Per ANA-10 (`docs/ANA-10.md` §4.8):
  `ready_items` must be **rewritten** for SQLite as well as implemented for Postgres — `<@` and
  `= ANY($projects)` have no SQLite spelling — which is why auto mode is not part of MOD-4's M8.
  Blocked on MOD-4.
- [ ] **MOD-13 - Backlog filters and item editing** (from MOD-1). `R-TUI-2`, `R-ENT-5`,
  `R-ENT-10..12`. Filters by status, project, capability and readiness; `new` and `edit` actions
  with the compare-and-set on `version` and the three-way divergence view (`docs/ANA-9.md` §4.2,
  §7.2), external `$EDITOR` round-trip, note thread append, hand-written documents; mint per §7.1.
  Not blocked (MOD-1 landed, `docs/decisions/mod/mod-1.md`); lands against `MemStore` through
  the `DetailRegistry` and overlay registry; `PgStore` (MOD-6, `docs/decisions/mod/mod-6.md`)
  supplies the real mint and revisions. The `touched_paths` edit path accepts and validates the
  `repo_name:glob` qualification of `docs/ANA-2.md` §4.7 (bare glob = primary repo);
  `touched_paths` is tier 1 of ANA-5's excerpt ranking (`docs/ANA-5.md` §4.5). **Editing on a box
  with no server does not exist** (MOD-25 closed 2026-09-16, `docs/decisions/mod/mod-25.md`): `htui`
  is online-only, so this item must not ship a `new`/`edit` action reachable while the backend is
  `Offline`, any local `mint_item` or `item_key_counter`, or a compare-and-set view backed by
  anything but `PgStore`/`MemStore`. An offline box browses read-only and says so.
  **The scope line "mint per §7.1" above means ANA-9 §7.1's Postgres statement**, which is now the
  only mint.
- [ ] **MOD-14 - Graph tab** (from MOD-1). `R-TUI-5`, `R-ENT-9`. Item neighbourhood one to N hops
  across projects through `ReadStore::links` (`docs/ANA-9.md` §6.1), status and link kind per
  edge, keyboard navigation that re-roots the Backlog selection; the `open graph` action of
  `R-TUI-2`. Not blocked (MOD-1 landed; replaces `ui/tabs/backlog/detail/graph.rs` only).
- [ ] **MOD-16 - Windows runtime verification of the agent driver** (from MOD-2). `R-AGT-1`,
  `R-NF-3`, `R-HIS-1`. Every Windows-only path MOD-2 compile- and lint-checked from Linux but
  never ran. `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent` is green and has already
  caught two defects a Linux build cannot see (`c4d65ba`), but four facts are runtime facts:
  the job object's kill-on-close guarantee leaves no `node`/`claude` process behind (`docs/ANA-4.md`
  §11 criterion 11's Windows half); `CreateProcess` refuses a `.cmd` shim, so `${claude}` resolving
  to one must fail with a message naming the shim rather than a bare `os error 193`;
  `CREATE_NO_WINDOW` actually suppresses the console; and milestone 4's `[H-1]` buffer sealing
  behaves under Windows rename semantics, where a sealed file held open by another process makes
  the rename fail rather than silently succeed (`seal_orphaned` must warn and carry on, and
  `seal_one`'s numbered fallback must not lose a tail). Also run the Postgres suites there: the
  cache is SQLite on a different filesystem, and `append_pending`'s `OpenOptions::append` and the
  `.jsonl.open` sealing have never met a Windows file lock. Needs a Windows box with `node` and
  `claude-agent-acp` installed; milestone 5 is the first milestone whose own work touches the
  spawn path, so this can run before or beside it. Not blocked, and **MOD-2 is now done across all
  nine milestones** (`docs/decisions/mod/mod-2.md`), so the full Windows surface is here rather than
  accumulating. Milestone 9 added one more runtime fact to the list: `htui_agent::excerpt::FsRepoReader`
  is the **first `htui-agent` code that traverses arbitrary repositories**, and its guarantees are
  `symlink_metadata` per component, `.git` pruned as a path component at any depth, and a scan cap
  counting every entry — none of which has met an NTFS junction, a reparse point or a case-insensitive
  path. Paths are repo-relative by contract (ANA-5 §4.2 rule 5) and the digest LF-normalises before
  any byte is counted (criterion 6), which is the Windows hazard that would otherwise change a
  `prompt_digest` between boxes. Criterion 11's CLI half also holds on **Linux only**.
  **MOD-20 added a second body of Windows-only code** (`docs/decisions/mod/mod-20.md`), and it is
  the first that could not be lint-checked from Linux at all (**TOOL-3**), so it was reviewed by eye
  only. The runtime facts it defers here, by name: that `Layout::promote`'s three-attempt
  `PermissionDenied` backoff actually clears a Defender lock on a freshly written `.exe`; long-path
  behaviour past 260 characters under `<install root>\<id>\<version>\`; that `create_link`'s
  Windows fallback (a small file naming the target, since a real symlink needs a privilege an
  ordinary user lacks) is never mistaken for the adapter; `fs4::available_space` on a junction;
  `reqwest`'s `system-proxy` reading the OS proxy configuration; and whether a partial
  `.staging/` entry can be removed while its file handle is open, which `fetch.rs`'s abandon path
  was restructured for but which no Linux test can distinguish.
  **MOD-21 added a third body of Windows-only code** (`docs/decisions/mod/mod-21.md`), unlinted for
  the same TOOL-3 reason and reviewed by eye. The runtime facts it defers here, by name: whether any
  Windows opener honours `BROWSER` at all and whether `cmd.exe /c exit 0` is a value it accepts —
  the neutraliser works only on an opener that *word-splits* the variable, and one that treats it as
  a single path falls through to exactly the stdout hijack the policy exists to stop, which was
  observed live on Linux; that `powershell.exe -NoProfile -NonInteractive -Command "Start-Process
  -FilePath $env:HTUI_OPEN_URL"` reaches the default browser under `CREATE_NO_WINDOW`
  (`ShellExecuteW` is unsafe FFI the workspace forbids, which is why the URL travels in the child's
  environment); and that the job object reaps the auth child **with its loopback listener** — MOD-2
  §11 criterion 11, now with a socket and a child that lives for minutes rather than seconds.

- [ ] **MOD-22 - Complete a loopback OAuth login from a box the browser cannot reach** (from MOD-21).
  `R-AGT-9`, `R-TUI-8`, `R-NF-3`, `R-SEC-2`, `R-ID-7`. An agent's own login flow redirects to a
  listener **inside the adapter process**, on that box's loopback: `agy_acp_server`'s URL carries
  `redirect_uri=http://127.0.0.1:<ephemeral port>/`, a different port per attempt (39879, then
  50651, measured live on 2026-09-09). When `htui` runs on the same machine as the browser this is
  invisible. When it runs on a **server**, the browser resolves `127.0.0.1` to the *user's* machine,
  nothing is listening, and the flow cannot be completed from the app at all - which is `R-AGT-9`'s
  own sentence left unfinished. Confirmed on this maintainer's setup 2026-09-10: of the two
  workarounds, **`ssh -L` port forwarding did not work and pasting the redirect back did** -
  copying the failed `http://127.0.0.1:<port>/?code=…&state=…` out of the browser's address bar and
  re-issuing it on the box (`curl`) hands the code to the adapter and `authenticate` returns.
  Scope: that paste-back, performed **in the TUI** rather than in a second terminal - a field in
  MOD-21's login pane that accepts the redirect URL the browser could not reach, validates it
  (loopback host, the port the flow actually advertised, `code`/`state` present), issues the request
  to that port **on the box `htui` runs on**, and reports what the listener said; the login then
  completes through MOD-21's existing path, so nothing here re-implements `authenticate`, the
  re-probe, or the outcome. Needs a text-input widget the Settings tab does not have yet
  (`R-TUI-8`), off the UI task like every other request (`R-NF-3`).
  **The URL is a credential-bearing value for the length of one request** - it carries an
  authorization `code` - so `R-SEC-2`/`R-ID-7` bind: it is never logged, never persisted, never put
  on a frame that outlives the request, and the pane must not echo it back after use. This is the
  one rule this item can most easily break, and MOD-21's `auth_live.rs` module doc is the precedent
  for stating it where the code is.
  Out of scope: changing what the agent binds (the vendor's, not `htui`'s), a `redirect_uri`
  override (not offered by ACP v1), and any general port-forwarding feature. Cross-links: **MOD-21**
  owns the login flow, its pane and its frames and is the shape to extend rather than duplicate;
  **MOD-16** owns whether the same paste-back works from a Windows box; **MOD-7**'s box registry is
  where "this box is remote" would eventually be a recorded fact rather than a guess. **Not
  blocked** — MOD-21 landed (`docs/decisions/mod/mod-21.md`). Found while running its live proof on
  2026-09-10.
- [ ] **MOD-23 - Agent registry editing in the Settings agents section** (from MOD-2). `R-AGT-4`,
  `R-AGT-6`, `R-TUI-8`. Create a manual agent row and edit an existing one: transport (`acp` or
  `cli`), launch command and args, model list, default model, billing mode, the `agent` row's
  `enabled` and the per-box `agent_box.enabled`. The surface is the **`agents` section of the
  Settings tab, not a tab of its own** — `SettingsSection`/`SectionId("agents")` in the section
  strip (`crates/htui/src/ui/tabs/settings/{mod.rs,agents.rs}`). It is **no longer the only
  registered section**: MOD-15 added `hierarchy`, `kinds`, `prompt` and `connection` after it, so
  the strip is five titles wide (46 of the pinned 100 columns, `tests/settings.rs`) and
  `SettingsSection::captures_input` now exists for a section that takes typed input. MOD-7's box
  profile and MOD-9's skills each add their own. MOD-2 built it read-only (`r` probe, `i` install, `a` authenticate, `o` open, `x` cancel)
  and MOD-20/MOD-21 added the install and login actions, so what is missing is the **write** half
  that `R-AGT-4`'s field list and `R-AGT-6`'s "manual entries allowed" still owe. MOD-2 milestone 5
  **D45** already added `probe.source` (`probe` | `manual`) so that "a manual entry is never
  overwritten by a probe that finds nothing" (`docs/ANA-4.md` §4.6) — a guard that today protects
  rows no UI can create. Needs the same text-input widget MOD-22 needs and the Settings tab does not
  have yet (`R-TUI-8`), off the UI task like every other request (`R-NF-3`). Registry writes are
  server-only (`REGISTRY_ON_SERVER_ONLY`, MOD-6 plan D52), and since MOD-25 that is the whole story
  — a box with no DSN browses read-only and edits nothing, so there is no second path to build here
  (`docs/decisions/mod/mod-25.md`; ANA-10's `R-AGT-4` amendment is withdrawn with the mode). **Not
  blocked**, and can start now. File collisions to expect in that one section file: MOD-2
  milestone 7 adds a quota column to this table (plan D73), MOD-12 owns the Settings caps section,
  MOD-15 **already landed** kinds and step graphs there (`docs/decisions/mod/mod-15.md`).
  **Budget warning inherited from MOD-2 milestone 7 (plan D76,
  T47/T48):** the table now runs eight columns with **no width slack left** — each sits at its own
  longest string (`transport`/`models`/`enabled` at their headers, `billing` at `subscription`,
  `default` at a 21-char model id, `quota` at `100% to 09-08`, `name` at `amp-acp`, `on this box` at
  `unauthenticated`) inside 98 usable columns. A ninth column costs a **ranking decision**, not an
  adjustment; an edit *pane* below the table, which this item needs anyway for its text input, is
  the cheaper shape than more columns. Out of scope: the caps editor (MOD-12), box-profile capability
  edits (MOD-7), and anything keyed on an agent's name (`R-AGT-5`). Raised by the maintainer on
  2026-09-10 while MOD-2 milestone 7 was in flight.
- [ ] **MOD-24 - Fault Tolerance of Agent Processes.** Implement agent memory checkpointing to Postgres. If the daemon or TUI crashes mid-run, `htui` should be able to read the last `SessionEvent` from Postgres, re-hydrate the agent's context window, and resume the exact step it was on so that multi-hour runs can survive process restarts.

### Deferred backlog

- [ ] **MOD-3 - Diff tab + code explorer.** `R-LATER-1`. Later tier; needs its own ANA first.
- [ ] **MOD-5 - Issue tracker mirror.** `R-LATER-2`. `IssueSync` trait, OneDev first, downstream
  only. Later tier; needs its own ANA first.

- [ ] **MOD-8 - Legacy markdown import.** `R-LATER-3`. Map old prefixes to kinds per project,
  preserve keys, build links. Later tier; MOD-6 landed (`docs/decisions/mod/mod-6.md`, importer
  mint variant per ANA-9 §7.1 still to write).

### Tooling findings

- [ ] **TOOL-3 - The Windows lint target cannot be built on this box, and MOD-20 made that bite.**
  `R-NF-3`, `R-AGT-1`. `cargo clippy --target x86_64-pc-windows-msvc` dies in `ring`'s build script
  with `error occurred in cc-rs: failed to find tool "lib.exe"`: cross-compiling `ring`'s C needs an
  MSVC-capable compiler, and this box has `gcc` only (`cc-rs` reports *"GNU compiler is not
  supported for this target"*; `AR_x86_64_pc_windows_msvc=llvm-lib` gets one step further and dies
  on *"not a COFF object"*). No `clang`, no `clang-cl`, no `cargo-xwin`, no `zig`, no `sudo`.
  **Pre-existing for `-p htui`** — `ring` reaches it as `ring ← rustls ← sqlx-core ← sqlx ←
  htui-core ← htui-store`, a path that predates MOD-20 — but MOD-20's `reqwest`/`rustls` brought it
  into `htui-agent`'s graph too, which is the crate the README's own command names
  (`README.md` "Checking the Windows-only code from Linux"). That command is the safety net MOD-2
  milestone 3 credited with catching two defects a Linux build cannot see, and it is now green on
  neither crate, so MOD-20's Windows-conditional code (`archive.rs`'s `cfg(unix)`/`cfg(not(unix))`
  arms, `Layout::promote`'s retry, the `canonicalize` on both sides of the post-promote check) was
  reviewed by eye rather than linted. Three ways out, none taken: install a C toolchain that can
  target MSVC without `sudo` (`cargo install cargo-zigbuild` + `pip install ziglang` is the
  sudo-free one; `cargo-xwin` needs `clang`); scope the lint line to a feature set that excludes
  TLS, which lints most code but not `install/http.rs`; or accept the loss and let **MOD-16** be the
  only Windows check. **This is a maintainer decision, and MOD-16 inherits the runtime half
  either way.** MOD-20's plan D21 and its Validation block both name lint lines that currently
  cannot run — whichever way this goes, they need amending. Found during MOD-20 T2 on 2026-09-09.
  being reaped before the assertion reads `/proc/<pid>`.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 4 (ANA-11 requirements/decisions models, ANA-16 execution environments, ANA-17 per-model prompt framing, ANA-21 per-model weights)                                 |
| MOD-N   | 23 (MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-16 Windows verification, MOD-22 loopback paste-back, MOD-23 agent registry editing, MOD-24 fault tolerance, MOD-26 personas, MOD-27 swarm, MOD-28 rataflow, MOD-31 preview blocks install, MOD-32 unscrubbed trim record, MOD-33 hostname out of the digest, MOD-34 Qdrant, MOD-36 weighted agent assignment; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                        |
| TOOL-N  | 1 (TOOL-3 Windows lint target unbuildable) |
