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

**Current status (2026-09-15):** **MOD-2 is done, all nine milestones**
(`docs/decisions/mod/mod-2.md`, `5c717d0`..`32e516d`): `htui` starts agent sessions, streams them
into a chat tab, answers permissions inline, persists and replays every event, tracks quota and
cancels on a cap — over **three transports** that pass **one** conformance list — and assembles the
prompt a step will be given, previewed read-only from the running binary. `R-AGT-5` is a passing
test, not a claim. Milestone 9 added the whole of `htui_core::prompt`, `htui-agent::excerpt`, five
store-seam methods (`CASES` 22 → **23**, `READ_CASES` 6), the sixth Backlog detail sub-tab
**Prompt**, and the Runs pane's `~36k`/`!` indicators. ANA-5 §12 criteria 1–20 each carry a named
test and criterion 21's five items are closed by D95–D99; criterion 18's `follow_up` persistence
half is **re-deferred to MOD-4 by name** (D107). `docs/ANA-5.md` §4.4/§5.1 are **amended in the
write-up, not in the file** (D108, maintainer-only per the milestone-5 precedent): the estimator is
`chars-v2`, Claude prose **2.5** / code **2.4** chars per token, *measured* — ANA-5's 3.5/3.0 was
28.4% off in the overflow direction and would have shipped silently. **1017 passed, 0 failed, 25
ignored** workspace-wide with Postgres live.
Before it, **ANA-15 concluded** (`docs/decisions/ana/ana-15.md`): Bugsink integration is feasible
via the standard Sentry SDK; spawned MOD-29. **ANA-3 concluded**
(`docs/decisions/ana/ana-3.md`): external context tools designed, manual grepping constrained.
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
**Concluded analyses the open items lean on:** ANA-10 (`docs/decisions/ana/ana-10.md`) — a box with
no DSN becomes a *complete* box over a separate `local.sqlite`, spawning MOD-17, MOD-18 and MOD-19,
and `docs/REQUIREMENTS.md` was amended for it on 2026-09-08 (new `R-STO-7`, eleven amended in
place); ANA-5 (`docs/decisions/ana/ana-5.md`) — the prompt contract, no new crate, no new migration;
ANA-2 (`docs/decisions/ana/ana-2.md`) — step graphs, three compare-and-set status tables,
`htui-orch`. MOD-7, MOD-9, MOD-13, MOD-14, MOD-15 and MOD-17 can start now.

---

## Open items

### Analyses


- [ ] **ANA-11 - Models for requirements and decisions.** Evaluate database schema models to track product requirements (R-IDs) and architectural decisions (MOD/ANA items) inside `htui` itself instead of standalone markdown files.
- [ ] **ANA-14 - Research whether using Redis could be beneficial.**
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

### Next features
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
- [ ] **MOD-30 - The detail sub-tab strip overflows at the pinned width** (from MOD-2, finding
  F-71). `R-TUI-3`. The sixth sub-tab took the strip past the pane: ` Body ` + ` Runs ` + ` Graph ` +
  ` Documents ` + ` Notes ` + ` Prompt ` is **45 columns** against a **43-column** inner pane at the
  harness's pinned 100×30, so `Prompt` renders clipped to ` Promp` and all 21 re-accepted snapshots
  carry the clip. Two candidate fixes, both cross-cutting, and **this is the maintainer's choice**:
  single-space separators in `detail/mod.rs::render_strip` (45 → **40** columns, and it changes every
  snapshot in `crates/htui/tests/snapshots/` plus the Settings and top-level strips if the rule is
  applied uniformly), or `DETAIL_PERCENT` 45 → 47 in `ui/tabs/backlog/mod.rs:30` (which widens the
  detail pane and narrows the backlog list, moving every backlog snapshot instead). A seventh sub-tab
  breaks whichever is chosen, so the fix should also pin the strip width against the pane in a test
  rather than leaving it to a re-accepted snapshot. Found during MOD-2 T67, 2026-09-14.
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
- [ ] **MOD-29 - Bugsink integration via Sentry crate (from ANA-15).** Add `sentry` and `sentry-anyhow` dependencies, configure DSN to Bugsink instance, and wire initialization.

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
  `set_step_prompt`, not `finish_step`. Per ANA-10 (`docs/ANA-10.md` §4.8, §9.1): **local-only mode
  runs graphs at parity**, so this item also owns M8 — `LocalStore` implementations of the seam above
  and the four SQL ports (`shared_serialized`'s advisory lock becomes an in-process lock sound only
  under ANA-10 §7.1's file lock, `FOR UPDATE` becomes `BEGIN IMMEDIATE`, `&&` over `repo_scope` and
  `<@` over tag arrays become Rust predicates shared with Postgres through `htui-core`). The
  `PgStore`-inherent reads of ANA-2 §8 become `Backend`-inherent with a four-arm `match self`
  (`agents()` precedent, `backend.rs:292-298`); §8's split rule itself is unchanged. Two
  prohibitions: never `CHECK (fanout_index >= 0)` on any of the three schemas (ANA-2 risk 12), and
  no local `shared_serialized` before ANA-10 §10.17 is answered. **No longer blocked on MOD-2**
  (done, `docs/decisions/mod/mod-2.md`); still blocked on **MOD-17's M3** (ANA-10 §9.1's ordering
  rule: `LocalStore` must exist before build step 1, or this item inherits eighteen unbudgeted
  methods), which MOD-25 removes when it closes (MOD-6 landed,
  `docs/decisions/mod/mod-6.md`). **Three things MOD-2 handed over by name.** (1) ANA-5 §12
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
  them into a project's `prompt_template` rows is MOD-15's (ANA-5 §4.6).
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
  `touched_paths` is tier 1 of ANA-5's excerpt ranking (`docs/ANA-5.md` §4.5). Editing on a box
  with no server is **MOD-17's** (ANA-10 concluded; `docs/ANA-10.md` §9.3): this item must not ship
  a `new`/`edit` action reachable while the backend is `Local` or `Offline`, any local `mint_item`
  or `item_key_counter` (that is MOD-17's M6, gated on the `R-ENT-7` amendment), a compare-and-set
  view backed by anything but `PgStore`/`MemStore`, or a second copy of the local-only status word.
  **The scope line "mint per §7.1" above means ANA-9 §7.1's Postgres statement**; on a box with no
  server the mint is ANA-10 §7.3's three-statement local form, which MOD-17 owns.
- [ ] **MOD-14 - Graph tab** (from MOD-1). `R-TUI-5`, `R-ENT-9`. Item neighbourhood one to N hops
  across projects through `ReadStore::links` (`docs/ANA-9.md` §6.1), status and link kind per
  edge, keyboard navigation that re-roots the Backlog selection; the `open graph` action of
  `R-TUI-2`. Not blocked (MOD-1 landed; replaces `ui/tabs/backlog/detail/graph.rs` only).
- [ ] **MOD-15 - Workspace, project, repo and kind management** (from MOD-1). `R-ENT-1..4`,
  `R-ENT-6`, `R-BOX-4`, `R-TUI-8`. Create and edit workspaces, projects (seeded kinds, graphs and
  templates per `docs/ANA-9.md` §5.10), repos with primary flag and per-box paths, workspace root
  paths per box; item kind editor with the prefix-change warning (§10); Settings tab sections for
  kinds and step graphs per project. Not blocked (MOD-6 landed, `docs/decisions/mod/mod-6.md`;
  `Settings > Rebuild cache` calls `CacheStore::rebuild()`). Seed graphs per `docs/ANA-2.md` §4.1
  (`review` in `implement`/`fix` `input_kinds`, `gate_hard` on `prd`, `plan` and `verdict`,
  `is_override = false`); whichever of MOD-15 and MOD-4 lands second owns the seed amendment. Per
  ANA-5 (`docs/ANA-5.md` §5.3, §5.4, §9): seed ten `prompt_template` rows per project (eight phase
  names plus reserved `judge` and `handoff`, amending `docs/ANA-9.md` §5.10), the phase editor
  refuses the two reserved names, and the Settings tab exposes the ten `app_setting` keys.
  **MOD-2 shipped the read half of those ten and none of the write half** (closed 2026-09-15,
  `docs/decisions/mod/mod-2.md`): `prompt::settings::{DEFAULTS, resolve_budget, resolve_reserve_bp,
  resolve_hops, resolve_max_skill_tokens, resolve_excerpt_caps}` resolve each key through
  phase → `project.settings` → `app_setting` → the compiled table, and `trim_record.budget_source`
  records which rung answered. There is **no `app_setting` writer on the store seam**, so today the
  only way to change `token_budget` (default **120 000**, with a 10% response reserve) or any other
  of the ten is SQL against the database — this item is what makes them editable from the app. The
  phase rung is the finest-grained one and has no editor either; `ResolvedPhase.token_budget` is
  read but never written. Creating
  a workspace on a box with no server is **MOD-17's** (ANA-10 concluded; `docs/ANA-10.md` §9.3,
  §9.4): "Not blocked" above holds for the server-backed paths only — the create paths for a
  server-less box depend on MOD-17. This item builds and registers the Settings connection
  **section** (`SectionId("connection")`, named by ANA-10 M0 so neither item has to guess);
  **MOD-25 owns the masked DSN field inside it** and must not have a second one built here. Also:
  no ad-hoc focus mechanism instead of M0's `TabAction::FocusSection` /
  `SettingsSection::captures_input` / `MaskedField` names; no second persistence location for a
  "shown once" marker or for local settings (ANA-10 §5.4's `local_setting` exists for that); and
  `Settings > Rebuild cache` stays as written — it is safe because the local store is a different
  file that `MIRRORED_TABLES` never names, so it must not be "helpfully" extended to clear local
  data, and its confirmation copy should say what it does and does not delete.

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

- [ ] **MOD-17 - Local-only mode: a writable local store** (from ANA-10).
  **Superseded by MOD-25 (maintainer decision, 2026-09-11): `htui` is online-only, so there is no
  writable local store. This line is deleted at MOD-25's close-out; nothing below it should be
  started.** `R-STO-1`, `R-STO-3..6`,
  `R-ENT-7`, `R-TUI-1`, `R-TUI-8`, `R-NF-3`, `R-ID-3`, `R-ID-7`, `R-HIS-1`, `R-USR-2`, `R-BOX-1`.
  A box that has never been given a DSN becomes a complete box rather than an empty read-only
  shell. Design concluded in `docs/ANA-10.md` (ANA-10, `docs/decisions/ana/ana-10.md`): a fourth
  `Backend::Local` arm over a **separate** file `<config_root>/local/local.sqlite` (never
  `cache.sqlite`, which is delete-on-mismatch), a new `LocalStore: WriteStore` with its own
  forward-only `local_migrations/` set, local id minting through a project-origin-scoped key
  counter, `box.toml` carrying three `#[serde(default)]` facts (no `local_only` mode flag), a
  once-only first-run overlay, and an in-app masked DSN field. **Local-only is a temporary, lite
  mode** (`docs/ANA-10.md` §1.3): parity is over *features* — hierarchy, items, chat, graph runs —
  not over server-shaped infrastructure, so there is no mirror, no refresh cursor, no
  `db_fingerprint`, no `offline · <age>` and no warm-cache startup budget on this path. Its
  first-run copy must not promise migration to a server before MOD-18 ships (§4.5's copy rule).
  Milestones **M0-M6 plus M2b** of
  `docs/ANA-10.md` §9.1; **M2b is the milestone that satisfies the item's stated objective** and
  M0+M1+M2+M2b is a complete shippable answer on its own (§9.2). Blocked on nothing, but **M3 must
  land before MOD-4's build step 1** (§9.1's ordering rule), and M6 is gated on the `R-ENT-7`
  amendment (`docs/ANA-10.md` §6.1, §10.3). MOD-13's, MOD-15's and **MOD-23's** create/edit paths
  for a server-less box depend on it; MOD-15 owns the Settings connection *section*, this item owns
  the credential *field* inside it (§9.4). The requirement amendments of §6.1 are proposed, not
  applied — `docs/REQUIREMENTS.md` is maintainer-only.
- [ ] **MOD-18 - Adoption of local rows into a server** (from ANA-10).
  **Superseded by MOD-25 (2026-09-11): with no local rows there is nothing to adopt. Deleted at
  MOD-25's close-out.** `R-STO-1`, `R-STO-7`,
  `R-ENT-7`, `R-USR-2`, `R-HIS-1`. `docs/ANA-10.md` §9.1's M7. **Funded, not deferred** (§10.5, on
  §1.3's framing): local-only is a temporary lite mode and Postgres is the full product, so this is
  the exit the mode promises rather than an optional extra. **MOD-17 must not ship first-run copy
  promising migration before this item exists** — until then the overlay says configuring a server
  does not move existing local work; when this lands, the copy gains the move. Explicit, confirmed,
  resumable; one transaction per project
  subtree; `MAX`/`GREATEST` counter fast-forward and the `sealed_at` write inside that transaction;
  `created_by`/`author_id`/`box_id` and `agent_id` remaps (§10.33); pre-adoption file copy;
  second-server refusal keyed on `system_identifier`; a UI-visible per-row status, never
  `pending.rs`'s log-only quarantine. Its unit is a project subtree including `run`, `run_step`,
  `session_event`, `document` and `command_run` rows, not only items. Blocked on MOD-17 (M6). Until
  it lands, local rows stay local and a local project is not runnable while a DSN is configured
  (`docs/ANA-10.md` §11 risk 22) — MOD-17's M3 export path is the floor. The `GREATEST` importer
  statement now has two consumers, MOD-8 and this item (`item.rs:145-148` reserves it to MOD-8).
- [ ] **MOD-19 - In-process transition out of local-only** (from ANA-10, deferred).
  **Superseded by MOD-25 (2026-09-11): there is no local-only mode to transition out of. Deleted at
  MOD-25's close-out.** `R-TUI-8`,
  `R-NF-3`. `docs/ANA-10.md` §9.1's M9 and §10.13: `connect::reconnect_for` as a public factory
  (the DSN stays inside `connect.rs`, which is `connect.rs:66-70`'s real property — the existing
  `Reconnect` closure already captures a DSN by move), `let mut reconnect` at
  `store_worker.rs:411`, and the expensive half, `go_online` learning to **open** a mirror rather
  than move one (`store_worker.rs:572-575`) while a `Backend::Local` and its open `local.sqlite`
  are still in hand. Buys one avoided restart and nothing else; MOD-17's M2b delivers the objective
  without it. Blocked on MOD-17 (M3).
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
  strip (`crates/htui/src/ui/tabs/settings/{mod.rs,agents.rs}`), which is still the *only*
  registered section (MOD-7's box profile, MOD-9's skills and MOD-15's hierarchy each add their
  own). MOD-2 built it read-only (`r` probe, `i` install, `a` authenticate, `o` open, `x` cancel)
  and MOD-20/MOD-21 added the install and login actions, so what is missing is the **write** half
  that `R-AGT-4`'s field list and `R-AGT-6`'s "manual entries allowed" still owe. MOD-2 milestone 5
  **D45** already added `probe.source` (`probe` | `manual`) so that "a manual entry is never
  overwritten by a probe that finds nothing" (`docs/ANA-4.md` §4.6) — a guard that today protects
  rows no UI can create. Needs the same text-input widget MOD-22 needs and the Settings tab does not
  have yet (`R-TUI-8`), off the UI task like every other request (`R-NF-3`). Registry writes are
  server-only (`REGISTRY_ON_SERVER_ONLY`, MOD-6 plan D52), so the same actions on a box with no DSN
  are **blocked on MOD-17** (ANA-10's `R-AGT-4` amendment, `docs/ANA-10.md` §6.1); the Postgres path
  is **not blocked** and can start now. File collisions to expect in that one section file: MOD-2
  milestone 7 adds a quota column to this table (plan D73), MOD-12 owns the Settings caps section,
  MOD-15 owns kinds and step graphs. **Budget warning inherited from MOD-2 milestone 7 (plan D76,
  T47/T48):** the table now runs eight columns with **no width slack left** — each sits at its own
  longest string (`transport`/`models`/`enabled` at their headers, `billing` at `subscription`,
  `default` at a 21-char model id, `quota` at `100% to 09-08`, `name` at `amp-acp`, `on this box` at
  `unauthenticated`) inside 98 usable columns. A ninth column costs a **ranking decision**, not an
  adjustment; an edit *pane* below the table, which this item needs anyway for its text input, is
  the cheaper shape than more columns. Out of scope: the caps editor (MOD-12), box-profile capability
  edits (MOD-7), and anything keyed on an agent's name (`R-AGT-5`). Raised by the maintainer on
  2026-09-10 while MOD-2 milestone 7 was in flight.
- [ ] **MOD-25 - `htui` is online-only** (maintainer decision, 2026-09-11). `R-STO-1`, `R-STO-3..7`,
  `R-ID-3`, `R-ENT-7`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`, `R-SKL-1`, `R-TUI-1`, `R-TUI-8`, `R-NF-2`.
  **A box that cannot reach its configured Postgres shows a warning that the database is
  unreachable and browses its read-only cache. It does not carry a second, writable life.** This
  withdraws the verdict of ANA-10 (`docs/decisions/ana/ana-10.md`) — "a box with no DSN becomes a
  *complete* box over a separate `local.sqlite`" — which is **concluded and therefore superseded
  rather than edited**. Scope, in order: (1) `docs/REQUIREMENTS.md`, the only maintainer-owned file
  here — withdraw **`R-STO-7`** outright and restate the eleven ANA-10 amended in place on
  2026-09-08 (`R-ID-3`, `R-ENT-7`, `R-STO-1`, `R-STO-4`, `R-STO-5`, `R-HIS-1`, `R-AGT-4`, `R-PRM-4`,
  `R-SKL-1`, `R-TUI-1`, `R-TUI-8`), of which `R-STO-4`'s offline-mode text changes meaning most;
  (2) **disable, do not delete**, the offline buffered-write path MOD-2 milestone 4 shipped
  (`Writer::Buffered`, `append_pending`, `upload_pending`, `crates/htui/tests/chat_offline.rs`) —
  a chat on an unreachable box refuses with the warning instead of buffering, and the machinery
  stays in the tree for one release so a reversal costs nothing; (3) a `CLEAN-N` minted at
  **this** item's close-out to remove it once the decision has sat. **The read-only cache of
  `R-STO-3` survives**: it is also the speed cache behind `R-STO-6`'s sub-second warm start, and
  degraded read-only browsing is what the warning is shown *over*. **Supersedes MOD-17, MOD-18 and
  MOD-19**, whose checklist lines this item's close-out deletes — they are not "done" and must not
  archive as if they were; `docs/decisions/mod/mod-25.md` records them as withdrawn with their
  reason. **Unblocks by removal**: MOD-4's build step 1 (ANA-10 §9.1 required MOD-17's M3 before
  it), and MOD-13's, MOD-15's and MOD-23's create/edit paths for a server-less box, which no longer
  exist. **MOD-2's close-out restated rather than silently dropped its claim on ANA-4 §11 criterion
  12** (an offline chat buffers and `upload_pending` lands exactly those rows), as this item asked:
  `docs/decisions/mod/mod-2.md`, "Known, accepted, and handed on" — it was proven end to end on
  2026-09-08 by `crates/htui/tests/chat_offline.rs` and is being withdrawn **with the mode**, which
  is a different sentence from "unproven". Disabling that path, and the CLEAN item that later
  deletes it, are still this item's.
  Not blocked. `docs/ANA-10.md` stays in the tree as the analysis that was done and not taken. **This item also owns the masked DSN field inside the Settings connection section, replacing MOD-17.**
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
| ANA-N   | 4 (ANA-11 requirements/decisions models, ANA-14 Redis research, ANA-16 execution environments, ANA-17 per-model prompt framing)                                 |
| MOD-N   | 28 (MOD-4 orchestrator, MOD-7 box, MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-15 hierarchy, MOD-16 Windows verification, MOD-22 loopback paste-back, MOD-23 agent registry editing, MOD-24 fault tolerance, MOD-25 online-only, MOD-26 personas, MOD-27 swarm, MOD-28 rataflow, MOD-29 Bugsink integration, MOD-30 detail strip overflow, MOD-31 preview blocks install, MOD-32 unscrubbed trim record, MOD-33 hostname out of the digest; **superseded by MOD-25 and deleted at its close-out: MOD-17 local-only store, MOD-18 adoption, MOD-19 in-process transition**; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0                                                                                        |
| TOOL-N  | 1 (TOOL-3 Windows lint target unbuildable) |
