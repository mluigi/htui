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

**Current status (2026-09-25):** **MOD-4 is done** (`docs/decisions/mod/mod-4.md`): the
orchestrator runs in manual mode across all six milestones. `htui-orch` walks step graphs with gates,
the review loop, judged fan-out and four git isolation modes under a leased heartbeat and a recovery
sweep, and `run_worker.rs` and the Runs pane let the maintainer drive it, promote a step to chat and
close an item out. Its carried risks are **MOD-37** and **CLEAN-4**.
Before it, **MOD-30 was done** (`docs/decisions/mod/mod-30.md`): the detail sub-tab strip separates
titles by one space and a test pins its width against the pane.
Before it, **MOD-35 was done** (`docs/decisions/mod/mod-35.md`): Qdrant connection settings mirror
the Postgres DSN configuration.
**Live coordinates.** The migrations are `0001_init`, `0002_agent_probe`, `0003_orchestration` and
`0004_max_agents_per_run_default` (cache: `0001`..`0003`), so **the next migration is `0005`**.
`max_agents_per_run` defaults to **8** (`0004` moves an untouched seeded `6`). Pins at MOD-4's close
(`22cfeea`): store conformance `CASES` 53, `READ_CASES` 9, `htui-orch` `CASES` 70, `StoreRequest`
62 variants, 227 `.sqlx` files; `cargo doc --workspace --no-deps` shows exactly two baseline errors
(`htui-core` `MIRRORED_TABLES`, `htui-store` `step_exists`). `git` ≥ 2.33.0 is a runtime dependency
of the `worktree` isolation mode and of reconciliation. **Production `approve` and `accept` are
greyed and every production judge fails until MOD-11**, because no agent can write its phase's
`output_kind` document yet, so production fan-outs go to the human (`s` in the Runs pane).
Adapters install under `HTUI_AGENTS_ROOT`, default
`dirs::data_local_dir()/htui/agents`; `HTUI_TOOL_<NAME>` still overrides everything. Dev Postgres
via `compose.yaml` (port 5439); tests need
`HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres` and the
`USERNAME=htui-ci` prefix of TOOL-2 (`docs/decisions/mod/mod-6.md`). Under load the dev Postgres
goes into recovery (`57P03`) and a failure seen then is re-run alone before it is believed;
`htui-store` `tests/cache.rs::the_spawned_refresher_passes_and_follows_the_scope` has timed out
once that way.
**Live coordinates the agent work left, kept here because open items depend on them.** `claude` on
this box is **2.1.267** (2.1.272 at the last estimator re-measure); the seed passes no `--bare`;
`--permission-prompts none` is the deterministic way to provoke a policy denial, and
`~/.claude/settings.json`'s allow list (`Bash(ls *)`) is why the obvious way does not. The CLI
reports `claude_code_version` on `system/init` (there is no `version` key), re-emits `system/init`
on **every turn** of a multi-turn session, and emits a `system/status` row per turn that
`docs/ANA-4.md` §6.2 does not name. This box's live quota blob is `status: "allowed_warning"` at
0.77 utilization; MOD-4 milestone 4 (D61) made `allowed_warning` selectable, so the `claude-cli`
row is no longer skipped for it. `agy_acp_server` **1.1.1** is installed here and emits **no `usage_update`
whatsoever**, which is why its seed keeps `quota.source: "none"` and why the GPT/Gemini estimator
row cannot be measured on this box (MOD-2 F-17). `--uid=` is **mandatory** for it. Its credentials
live in `$GEMINI_HOME/antigravity-acp/acp_token.json`, a sibling of and separate from the `agy`
CLI's own directory (MOD-21).
**Concluded analyses the open items lean on:** ANA-10 (`docs/decisions/ana/ana-10.md`) — **its
verdict is withdrawn**, see MOD-25 (`docs/decisions/mod/mod-25.md`); the document stays in the tree
as the analysis that was done and not taken, and anything leaning on its local-only half is stale;
ANA-5 (`docs/decisions/ana/ana-5.md`) — the prompt contract, no new crate, no new migration;
ANA-2 (`docs/decisions/ana/ana-2.md`) — step graphs, three compare-and-set status tables,
`htui-orch`, now built (MOD-4, `docs/decisions/mod/mod-4.md`). MOD-7, MOD-9, MOD-11, MOD-12, MOD-13
and MOD-14 can start now (MOD-15 is done, `docs/decisions/mod/mod-15.md`).

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
  milestone 4, OQ-7; maintainer-requested 2026-09-23; MOD-4 is done, `docs/decisions/mod/mod-4.md`).
  `R-AGT-8`, `R-ORCH-7`. Blocks MOD-36.
  Research how to give each configured agent/model a weight (e.g. Gemini Flash 30, Claude Opus 5.5
  60) that MOD-36 uses to choose which models run a phase's fan-out candidates. Survey public signals
  (coding and reasoning benchmarks, leaderboards, published pricing and latency), decide whether
  weights are per task kind (analysis, implement, review, judge) or global, and whether cost enters
  the weight or stays a separate quota concern (ANA-4). Deliver: the weight table's schema and where
  it lives (`agent` row, `app_setting`, or a new table), a refresh method (manual, scripted from
  named sources, or learned from htui's own judge verdicts), and initial values for the seeded agents.

### Next features
- [ ] **MOD-37 - Orchestrator hardening follow-ups** (from MOD-4). `R-ORCH-3`, `R-ORCH-5`,
  `R-ORCH-8`, `R-ORCH-9`, `R-TUI-4`, `R-HIS-1`, `R-NF-3`. MOD-4 closed with these risks carried and
  no other item owns them. Each is small, known and recorded; none blocks a manual run today. Pick
  them off singly or in batches. Sources are under `.claude/plans/mod-4-orch-*`, and the context is
  in the MOD-4 write-up's "Carried" section (`docs/decisions/mod/mod-4.md`).
  - **R-3**: a parked run's `run.failure` stays NULL. The reason lives only in the step's
    `gate_note` and the `item_note`, and the Runs pane shows `awaiting_approval` without it, because
    adding `gate_note` to `RunStepSummary` touches three builders and the mirror (engine blueprint
    F-I; drive plan, "What this milestone touches").
  - **R-5**: the gate park is three compare-and-sets (step, run, item), not one transaction on
    Postgres; milestone 5's D96 closed its crash half. Nothing writes `gate_outcome = skipped`, so a
    `never`/`on_failure` pass leaves the gate NULL and the step list renders `—` (engine blueprint
    F-K, H-9, H-10; drive plan, "What this milestone touches").
  - **R-6**: nothing writes `phase_agent` for an override graph copy, or `is_override`. The plans
    named MOD-15's phase editor as its owner, but MOD-15 closed on 2026-09-17, so it lands here
    (engine blueprint F-J; drive plan, "What this milestone touches").
  - **R-29**: Postgres stores `queued_at` in microseconds and `MemStore` in nanoseconds, so a
    sub-microsecond tie can name a different `Overlaps.with` (lease blueprint §21.2).
  - **R-30**: `recover::classify` counts a step as finished only when every `run_scope` repo has an
    `after_hash`. A step that changed only some repos and crashed after capture is retried rather
    than adopted. The failure is safe, a retry and never a wrong merge (lease blueprint §21.2).
  - **R-31, the rejected-crash remainder**: a crash right after `AnswerGate(Rejected)` stays parked
    on a failed step with no resume path. The rest of R-31 was closed by milestone 6's D180 (lease
    blueprint §22.3; drive plan D180).
  - **R-32**: D131's not-reset park loses its labelled detail, and D138's `part_way` turns an `Io`
    error into a `Git` error. Both are diagnostics only (lease blueprint §22.3).
  - **R-37**: `git::reconcile_parent` opens the checkout up to six times per diff row, and `merge_of`
    walks the primary's first-parent history back to the step's base. This costs speed only. The
    fix is to open the repository once and pass `&gix::Repository` to private `*_in` variants (lease
    blueprint §23.4).
  - **R-38**: preempting a running step (`cancel`, `promote`) kills the agent without ANA-4 §4.3's
    grace window and without answering parked permission requests. A graceful path needs a cancel
    seam inside `pump` (drive plan, Risks).
  - **R-40**: `RunStream` frames are sent at session end and at rest only, so a step's
    `pending → running` is not signalled to the Runs pane. The next frame, or re-selecting the item,
    shows it. The fix is a step-start hook (drive plan, Risks).
  - **R-41**: an `Orch` reply can arrive hours after its request, and a newer `Orch` request from
    the same origin makes it stale, so it is dropped (`App::is_fresh`). The walk's result still
    reaches the pane as a `RunStream` frame and through the rows (drive plan, Risks).
  - **R-44**: step rows at 43 columns truncate `agent/model` for long model ids. `…` marks the cut
    and the width test keeps it from clipping silently (drive plan, Risks).
  - **R-46**: a walk task keeps the `Backend` clone it started with. After an `Online → Offline`
    swap its `PgStore` handle keeps failing until the heartbeat fences, and the sweep after reconnect
    adopts the run (drive plan, Risks).
  - **R-48**: the ACP driver ignores `SessionSpec.resume` (only `cli/mod.rs` reads it), so a
    promoted ACP step always gets the handoff prompt and a fresh model context. It needs ACP
    `session/load`. The blueprint named "a MOD-2 follow-up" as the owner, and MOD-2 is closed (drive
    blueprint §18).
  - **R-49**: a promoted chat works in the step's tree without the `shared_serialized` `(box, repo)`
    guard. The guard was released at `capture` or by `abandoned`, so another run may `prepare` the
    same checkout meanwhile. The fix is to take the guard again in `attach_promoted` (drive
    blueprint §18).
  - **R-51**: a command on a run with a live walk waits for the whole walk (D157), and the pane shows
    nothing while it waits. The fix is a "waiting" frame (drive blueprint §18).
  - **R-53**: `ItemActions` is as of the last `Runs` reply, so a verdict can flip before the key is
    pressed. The engine re-checks with the same admission function (D184) and the pane re-reads
    (D171) (drive blueprint §18).
  - **R-55**: `box.settings.command_limits` is read once per process, per server, so an edit does not
    reach a running process's verifier until a restart or a server switch. Nothing edits it today;
    whoever adds an editor re-reads the limits or rebuilds the verifier when no walk is live (drive
    blueprint §21.3).
  - **T7's residual window**: a promoted chat first streams at the promotion's `Orch` address and
    moves to its own once the Chat tab's `ChatFollow` is served. Between the chat's `ChatAccepted`
    and that `ChatFollow`, a second `Orch` request from the Chat tab supersedes the address, so the
    frames sent in between are dropped from the view as stale while the store keeps recording them.
    A promotion refused at the bind is already handed the stream (blueprint D185); the interval
    before the follow is served is not covered (T7 repair `9f7cc5c`, `agent_worker.rs` `Stream`).
- [ ] **MOD-36 - Weighted agent assignment across fan-out candidates** (from MOD-4 milestone 4,
  OQ-7; blocked on ANA-21 only, since MOD-4 is done, `docs/decisions/mod/mod-4.md`). `R-AGT-8`,
  `R-ORCH-7`. Milestone 4 runs every candidate of a group on the
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
  against the preview's; MOD-4 is done, `docs/decisions/mod/mod-4.md`) the moment the two run on
  different boxes. The hostname stays *in the text the model sees*; it is excluded from the bytes
  that are hashed. That split does not exist yet:
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
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees. `htui-orch`, its `Isolator` seam and `run_worker.rs` exist since MOD-4 (done, `docs/decisions/mod/mod-4.md`); the MCP half needs MOD-11.
- [ ] **MOD-7 - Box registry + capabilities.** `R-BOX-1..4`, `R-ORCH-10`, `R-AGT-6`, `R-TUI-8`.
  Probe, registration, capability tags and quirks editor (Settings tab box profile section),
  per-box paths, agent autodiscovery hook. Not blocked (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`: `register_box` writes the minimal row, the probe fills the rest).
  MOD-12 needs `probed_tags`/`declared_tags` from a real probe, `repo_box_path` rows for every
  isolation mode, and `agent_box.probe.status` (`docs/ANA-2.md` §4.10, §7). MOD-4 is done
  (`docs/decisions/mod/mod-4.md`) and shipped without them: `R-ORCH-10`'s capability refusal, and
  therefore ANA-2 criterion 14's capability half, are this item's (MOD-4 proved criterion 14's
  `Unblock` half over a no-candidate refusal instead), and the production isolator reads
  `repo_box_path` through `Backend::repo_paths(box)`. `repo_box_path`
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
  the one **missing** call site is **MOD-32**, not this item. **MOD-4 wired no secrets** (done,
  `docs/decisions/mod/mod-4.md`, plan D176): `htui-orch`'s `drive_once` builds every graph
  `SessionSpec` with an empty `env`, and its comment names this item as the one that fills it.
- [ ] **MOD-11 - htui MCP server.** `R-MCP-1..4`. Tools `item_link`, `item_status`,
  `document_write`, `note_add`, `box_profile`, `command_run`; per-step scoping; command queue with
  per-box class limits; per-phase exposure. Per ANA-2 (`docs/ANA-2.md` §4.2, §8, risk 11):
  `document_write` calls `WriteStore::write_document` (orchestrator-allocated version), `command_run`
  accepts class `verify` for `verify_command`, and an `item_status` request is recorded as an
  `item_note` with `via_step_id`, never a transition. The `box_profile` read tool returns ANA-5's
  box profile projection (`docs/ANA-5.md` §4.2) so the tool and the prompt section agree — MOD-2
  shipped that projection as `htui_core::prompt::BoxProfile::project`, which drops `box_tool.path`
  (ANA-5 §4.2 rule 5), so the tool must not re-add it. **Not blocked**: MOD-2 and MOD-4 are done
  (`docs/decisions/mod/mod-2.md`, `docs/decisions/mod/mod-4.md`). MOD-2's own `permission_request`
  gap over the CLI transport stays declared until this item lands, since the
  `--permission-prompt-tool` contract is its route. **MOD-4 left two things waiting here**: every
  production judge fails until an agent can write the `judge` document (MOD-4 milestone 4, OQ-4),
  and production `approve`/`accept` are greyed in the Runs pane because production's `SessionSink`
  is `NoSink`, so no step writes its `output_kind` document outside a test (MOD-4 risk R-50). Both
  clear once `document_write` exists.
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
  **Not blocked**: MOD-4 is done (`docs/decisions/mod/mod-4.md`); `claim_run`'s admission, the
  overlap predicate, the lease, the sweep and `run_worker.rs` are there to reuse.
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
  only mint. MOD-4's close-out writes a generated `summary` document with no human prose (MOD-4 risk
  R-43, `docs/decisions/mod/mod-4.md`); an editable summary is this item's editor's.
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
  **MOD-4 handed over two more** (`docs/decisions/mod/mod-4.md`). **R-10**: a SIGKILLed orchestrator's
  agent survives, because the child leads its own process group and `ChildGuard::drop` cannot run in
  a killed process; signalling a stale pid needs `unsafe` or a dependency, so the design (a pid in
  `session_started` plus a signal on adoption, or a death signal, with the reused-pid hazard stated)
  is this item's. **R-45**: `htui` now links `gix`, `process-wrap` and `walkdir` through `htui-orch`.
  MOD-4 also made the `git` CLI (≥ 2.33.0) a runtime dependency of the `worktree` mode and of
  reconciliation, and none of those `git` calls has run on Windows.

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

- [ ] **CLEAN-4 - `LoopStop::NoProgressReview` is unreachable** (from MOD-4, risk R-9). `R-ORCH-3`.
  The review loop's no-progress predicate has two halves, and only the `after_hash` half can fire.
  `gate::reviews_are_identical` reads through the latest-only `documents_of_kinds`, so it can never
  hold two review documents to compare, and `LoopStop::NoProgressReview` has been unreachable since
  milestone 2. No test reaches it; only its `Display` is tested. Fix it through `documents()` heads,
  in a change that first pins which stop reason each shipped loop case reaches, because the fix can
  change that. Source: `.claude/plans/mod-4-orch-fanout.blueprint.md` F-B and §11, carried unchanged
  through milestones 5 and 6 (`docs/decisions/mod/mod-4.md`, "Carried").
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
| MOD-N   | 23 (MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-16 Windows verification, MOD-22 loopback paste-back, MOD-23 agent registry editing, MOD-24 fault tolerance, MOD-26 personas, MOD-27 swarm, MOD-28 rataflow, MOD-31 preview blocks install, MOD-32 unscrubbed trim record, MOD-33 hostname out of the digest, MOD-34 Qdrant, MOD-36 weighted agent assignment, MOD-37 orchestrator hardening; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 1 (CLEAN-4 unreachable `NoProgressReview`)                                               |
| TOOL-N  | 1 (TOOL-3 Windows lint target unbuildable) |
