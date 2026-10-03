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

**Current status (2026-10-03):** **MOD-71 is done** (`docs/decisions/mod/mod-71.md`): the Runs
flow view takes the mouse. A click on a node moves the shared cursor, a drag on empty canvas pans,
and the wheel zooms at the pointer; nodes stay read-only. Capture is on only while that view is
shown in browse mode with a run to draw. Everywhere else htui is keyboard-only and the terminal
keeps its own text selection (in the flow view, the terminal's bypass modifier, usually Shift,
still selects). Every terminal give-back (restore, panic hook, `$EDITOR`) turns capture off. A pan
survives the active-run poll. No migration, no new crate (`rataflow` gained its `crossterm`
feature). Follow-ups are MOD-74.
Before it, **MOD-13 is done** (`docs/decisions/mod/mod-13.md`): the Backlog
filters and edits items. `f`/`F` filter by status, project, tags and "ready here"; `N`/`e` mint and
edit through ANA-9 §7.1/§7.2, and a stale edit opens a three-way divergence view that resolves to a
`divergence_resolution` revision; Ctrl+E hands long text to `$EDITOR`. Milestone 5 made the Notes
and Docs sub-tabs writable: `a` adds a note, and in Docs `a` writes a hand-written document of any
kind (an editable `summary` included) and `v` a new version prefilled from the latest. Every write
is refused offline before any read; a write whose answer may have been lost settles itself on a
re-read and never invites a duplicate. Conformance case 135; no migration. It spawned **MOD-73**
(hand-written versions as step inputs).
Before it, **MOD-72 is done** (`docs/decisions/mod/mod-72.md`): every node of
the Runs flow view has a third line counting its step's tool calls by kind (`⚒ read×5 exec×3 +1`).
`ReadStore::tool_call_counts` answers on Postgres, the mirror and `MemStore`; the pane asks
`StoreRequest::ToolCalls` after each `Runs` reply only while the flow is shown. Nodes are 20×5. No
migration, no new crate, one new `.sqlx` entry.
Before it, **MOD-60 is done** (`docs/decisions/mod/mod-60.md`): every
hand-laid-out row in the TUI measures, pads, clips and wraps in terminal cells, the way ratatui draws
them, so CJK, emoji, combining marks and halfwidth kana no longer overrun a pane or push a column out
of line. `ui::cells` gained `clip`/`pad`/`pad_left`/`fit`/`wrap`/`clip_spans` and ~16 local copies
went; `cell_width` is now a per-grapheme sum (B10); the requirements tree elides a narrow project
name before its ` · read-only` marker; the MOD-13 divergence columns stay fixed-width. No migration,
no new crate, no snapshot moved.
Earlier completions are in `DECISIONS.md`.
**Live coordinates.** The migrations are `0001_init`, `0002_agent_probe`, `0003_orchestration`,
`0004_max_agents_per_run_default`, `0005_box_identity` (MOD-7 milestone 1), `0006_requirements`
(MOD-38), `0007_skill_attachments` (MOD-9 milestone 2), `0008_trim_record_v3` (MOD-9 milestone 5,
comment only) `0009_agent_box_user_off` (MOD-23), `0010_prompt_digest_undigested` (MOD-33, comment only)
`0011_permission_relay` (MOD-42), `0012_persona` (MOD-26 milestone 1) and `0013_persona_phase_index`
(MOD-26 milestone 2, index only; cache: `0001`..`0004`), so **the next migration is `0014`** (cache: `0005`).
`max_agents_per_run` defaults to **8** (`0004` moves an untouched seeded `6`). Pins after MOD-7
(done, all four milestones), MOD-38, MOD-9 (done, all five milestones), MOD-40, MOD-39, MOD-64,
MOD-23, MOD-22, MOD-41, MOD-59, MOD-24, MOD-49, MOD-28, MOD-72 and MOD-26 (done, both milestones)
(re-counted 2026-10-01; `TABLES` and commented columns 2026-10-02; `CASES`, `READ_CASES`, `htui-orch`
`CASES`, `StoreRequest`/`StoreReply`, the `REQUEST_NAMES` below, `.sqlx`, snapshots and Settings
sections 2026-10-03): store conformance `CASES` 134, `READ_CASES` 14, `htui-orch` `CASES` 92,
`GraphSource` 7 methods, `StoreRequest` 105, `StoreReply` 62, `AuthFrame` 11, `hierarchy::REQUEST_NAMES` 13,
`skills::REQUEST_NAMES` 6, `persona_settings::REQUEST_NAMES` 5, `TABLES` 42, 322 `.sqlx` files, 143
`crates/htui/tests/snapshots`, six workspace members (`htui-worker` since MOD-41),
`MIRRORED_TABLES` 21, eight Settings sections (71 of the 100 strip columns), 44 pinned commented
columns (`tests/migrations.rs`), and `run_step.trim_record` at `v: 4` (MOD-33 `undigested`) with `skill_choices` (a
`matched` choice carries `path`, `<repo>:<path>`); `Isolator` gained `changed_paths` (MOD-9 D119).
Excerpts reach phase prompts since MOD-7 milestone 4, so a phase-prompt digest recorded before
2026-09-26 does not compare with a later one; since MOD-9 milestone 5 (2026-09-30) a matched `glob`
skill renders and a retry's excerpts rank the previous attempt's changed paths, so those digests
move too; handoff digests are unchanged. Since MOD-33 (2026-09-30) a phase prompt digests
`[hostname]` in place of the box's hostname, so every box-bearing phase digest moved once more.
`cargo doc --workspace --no-deps --keep-going` shows exactly six baseline errors (`htui-core`
`MIRRORED_TABLES`; `htui-store` `step_exists`, `HashEmbedder` in `embed.rs`, and three private
links MOD-38 added in `pg/write.rs`: `set_requirement_spec` to `cas_miss`, `amend_requirement` and
`withdraw_requirement` to `revise_requirement`; the count read five before 2026-09-26, but
`HashEmbedder` already failed at `98e6d2f`). With `-D warnings`, `cargo doc` also fails on four
pre-existing private-item links (`store/traits.rs:1251`, `agent_worker.rs:729`,
`ui/text_area.rs:18`, `ui/text_field.rs:5`; recorded at MOD-9's close, `docs/decisions/mod/mod-9.md`). `git` ≥ 2.33.0 is a runtime dependency
of the `worktree` isolation mode and of reconciliation. **Production `approve` and `accept` are
greyed and every production judge fails until MOD-11**, because no agent can write its phase's
`output_kind` document yet, so production fan-outs go to the human (`s` in the Runs pane).
Adapters install under `HTUI_AGENTS_ROOT`, default
`dirs::data_local_dir()/htui/agents`; `HTUI_TOOL_<NAME>` still overrides everything. Dev Postgres
via `compose.yaml` (port 5439, loopback only since TOOL-7); tests need
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
ANA-5 (`docs/decisions/ana/ana-5.md`) — the prompt contract, no new crate, no new migration, with
ANA-17 (`docs/decisions/ana/ana-17.md`) settling its block separator and keeping one frame for
every model;
ANA-2 (`docs/decisions/ana/ana-2.md`) — step graphs, three compare-and-set status tables,
`htui-orch`, now built (MOD-4, `docs/decisions/mod/mod-4.md`). MOD-11 and MOD-12 can start
now (MOD-13 is done, `docs/decisions/mod/mod-13.md`; MOD-14 is done, `docs/decisions/mod/mod-14.md`; MOD-15 is done, `docs/decisions/mod/mod-15.md`; MOD-7 is done,
`docs/decisions/mod/mod-7.md`; MOD-9 is done, `docs/decisions/mod/mod-9.md`).

---

## Open items

### Analyses


- [ ] **ANA-25 - Learn per-model weights from htui's own judge verdicts** (from ANA-21; ANA-21 is
  done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`, `R-ORCH-7`, `R-ID-6`.
  ANA-21 deferred learned weights behind a volume trigger but the maintainer asked for it to be
  tracked rather than remembered. **Trigger: do not start until MOD-36 and MOD-11 are both done and
  a project holds on the order of 100 judged cross-model groups.** The data is already recorded —
  candidate `run_step` rows carry `agent_id`, `model`, `selected` and `verify_outcome`, and the
  judge row (`fanout_index = -1`) carries its own `agent_id`/`model` — so no new logging is needed.
  Fit a Bayesian Bradley-Terry model with a Plackett-Luce top-1 likelihood per verdict, taking
  ANA-21's tier as the prior and a same-family-as-judge covariate, since the documented self- and
  same-family preference (+10 to +25 points self, +3.4 to +8.4 same family) is as large as the
  signal being measured. Show the fit read-only beside the manual weights before anything is applied
  automatically. The fit is deterministic arithmetic over rows, so it is allowed under `R-ID-6`.
  Judge choice itself is out of scope: ANA-21 §2.2 established the judge is never asked of the
  selector, and MOD-36 owns the judge-identity hardening.
### Next features
- [ ] **MOD-37 - Orchestrator hardening follow-ups** (from MOD-4). `R-ORCH-3`, `R-ORCH-5`,
  `R-ORCH-8`, `R-ORCH-9`, `R-TUI-4`, `R-HIS-1`, `R-NF-3`. MOD-4 closed with these risks carried and
  no other item owns them. Each is small, known and recorded; none blocks a manual run today. Pick
  them off singly or in batches. Sources are under `.claude/plans/mod-4-orch-*`, and the context is
  in the MOD-4 write-up's "Carried" section (`docs/decisions/mod/mod-4.md`).
  PRD `.claude/prds/mod-37-orchestrator-hardening.prd.md`, five milestones.
  **Phase 1 landed (`19ca229`..`151c87e`, 2026-10-02):** run state and visibility. R-3, R-40,
  R-41, R-51 and T7 are closed. R-44 and R-53 are re-deferred with reasons. Plan and blueprint are
  `.claude/plans/mod-37-run-state.plan.md` and `.claude/plans/mod-37-run-state.blueprint.md`. The review left one LOW unfixed: the Chat
  tab's `followed` stays set after a failed bind until the next `Promoted` overwrites it, which is
  harmless.
  **Phase 2 landed (`1d1118f3`..`7df69d33`, 2026-10-02):** store and engine correctness. R-5, R-6,
  R-29, R-30 and the R-31 remainder are closed; R-32 is half closed (D138's `part_way`) and its
  D131 half is re-deferred. Plan and blueprint are `.claude/plans/mod-37-store-engine.plan.md` and
  `.claude/plans/mod-37-store-engine.blueprint.md`. Maintainer-accepted side effect of R-31:
  `Unblock` on a followed review-loop escalation now resumes it, re-runs the loop and, with nothing
  changed, escalates again (it used to be refused); a test pins it. rust-reviewer approved; its
  three LOWs and one NIT were applied. Two NITs were left: `unblock_enabled`'s bare `bool` and a
  `Vec<String>` copy in the `phase_agent` insert.
  **Phase 3 landed (`7cf455a8`..`a387249c`, 2026-10-02):** git cost. R-37 is closed. Plan and
  blueprint are `.claude/plans/mod-37-git-cost.plan.md` and
  `.claude/plans/mod-37-git-cost.blueprint.md`. rust-reviewer approved; its two LOWs and three NITs
  were applied, and one NIT (a hash parsed twice, which is negligible) was left. Not in R-37 and not
  changed: `RealIsolator`'s reconcile still opens the checkout once per `blocking` hop
  (`isolate/real.rs`, `head`, `merge_of`, `is_ancestor` twice) under the admin lock.
  **Phase 4 landed (`68a7c7e9`..`1787fd69`, 2026-10-03):** deadline and sessions. The ANA-27 T4
  deadline, R-46 and R-49 are closed. Plan and blueprint are
  `.claude/plans/mod-37-deadline-sessions.plan.md` and
  `.claude/plans/mod-37-deadline-sessions.blueprint.md`. R-49 was closed by a pin, not a guard
  (maintainer amendment): `claim_run`'s overlap admission already keeps other runs off the repo.
  R-46 preempts only on the refresher's verdict, because a TUI read that fails `Unreachable`
  includes a `PoolTimedOut` under local load. rust-reviewer approved with fixes; its two MEDIUMs,
  three LOWs and one NIT were applied, and one NIT (a `Debug`-string comparison in a pin test) was
  left.
  - ~~**R-3**~~: closed by MOD-37 phase 1. `RunStepSummary` carries `gate_note` from all three
    builders (Mem, Pg, cache mirror), and the Runs pane shows a parked step's reason on a third
    line. `run.failure` stays NULL on a park, as before.
  - ~~**R-5**~~: closed by MOD-37 phase 2. The gate parks through one fenced `park_step` (step, run
    and item in one transaction, `promote_step`'s shape), and a `never`/`on_failure` pass goes
    through `pass_step`, which writes `gate_outcome = 'skipped'` as ANA-2 §4.2's table says. D96's
    recovery stays for rows written before.
  - ~~**R-6**~~: closed by MOD-37 phase 2. `WriteStore::create_phase_agents` writes a phase's agent
    rows, `override_graph` copies each phase's agents onto its clone, and `MemStore` now holds
    `phase_agent` rows instead of always answering empty.
  - ~~**R-29**~~: closed by MOD-37 phase 2. `MemStore` truncates a graph run's `queued_at` to the
    microsecond, as Postgres does, so a tie inside one microsecond breaks on `id` in both stores.
    `open_chat` still stores a chat run's `queued_at`/`started_at` untruncated (review NIT, outside
    R-29's graph-run scope).
  - ~~**R-30**~~: closed by MOD-37 phase 2. `classify` adopts a step whose capture changed only some
    repos: one `after_hash` on a scope repo proves the one-transaction capture landed. Candidates
    keep the every-repo rule (a failing candidate's trees are captured too), and a step that changed
    nothing and crashed before `finish_step` is still retried, safely.
  - ~~**R-31, the rejected-crash remainder**~~: closed by MOD-37 phase 2. `status::resumable`
    (D196's one predicate, widened) treats a parked run over a `failed` + `rejected` step as
    resumable, so `Unblock` unparks it and runs the rejection's tail; the worker-box hand-back does
    the same. Side effect accepted by the maintainer: see the phase 2 note.
  - **R-32** (D138 half closed by MOD-37 phase 2: `part_way` keeps an `Io` error as `Io`, kind
    included): D131's not-reset park loses its detail. Re-deferred: `labelled` is always empty on a
    refusal, and the lost part is the reason text, which only `never_reset` holds in memory;
    keeping it needs a new column or a change to `gate_note`, which the engine and four tests match
    exactly. Diagnostics only (lease blueprint §22.3).
  - ~~**R-37**~~: closed by MOD-37 phase 3. `reconcile_parent` opens the checkout once and hands
    the `gix::Repository` to private workers; the public path-taking functions keep their
    signatures, their error order and their text. A test-only open counter pins it (6 opens before,
    1 after). `merge_of`'s first-parent walk down to the base stays: it is D136's semantics, and
    D142 already bounds it.
  - ~~**R-38**~~: closed by MOD-42 (`docs/decisions/mod/mod-42.md`): a `cancel` or `promote`
    preempt signals the walk, which answers parked requests `cancelled` and cancels the session
    with grace before the walk is dropped.
  - ~~**R-40**~~: closed by MOD-37 phase 1. `SessionSink::started` runs after each
    `pending → running` move, and `ProgressSink` publishes `FrameKind::Changed` for it.
  - ~~**R-41**~~: closed by MOD-37 phase 1. A `RunStream` `Error` frame puts its sentence on the
    status line even when the `Orch` reply that carried the failure was dropped as stale.
  - **R-44** (re-deferred by MOD-37 phase 1): step rows at 43 columns truncate `agent/model` for
    long model ids. `…` marks the cut and the width test keeps it from clipping silently (drive
    plan, Risks). Any real fix is a layout decision (drop the indent, abbreviate the model, or add
    a line), so it waits for the Runs pane's next layout change.
  - ~~**R-46**~~: closed by MOD-37 phase 4. When the refresher reports the server gone, the
    store loop calls `RunRuntime::preempt_walks`, so every live walk is abandoned at once and the
    sweep after reconnect adopts it as a new attempt. Previously it ran blind for up to about 80 s,
    until the heartbeat fence. A short blip now ends the session (maintainer decision). A loss
    that a TUI read notices first does not preempt, because the read can be a pool timeout with the
    server up and `go_offline` stops the refresher. Those walks keep the heartbeat-fence path.
  - **R-48**: the ACP driver ignores `SessionSpec.resume` (only `cli/mod.rs` reads it), so a
    promoted ACP step always gets the handoff prompt and a fresh model context. It needs ACP
    `session/load`. The blueprint named "a MOD-2 follow-up" as the owner, and MOD-2 is closed (drive
    blueprint §18).
    ANA-27 (`docs/ANA-27.md` §5.1 T5): until ACP `session/load` lands, a promotion that was not
    resumed says so. When `promote::opening_kind` chooses `Handoff`, or when the CLI driver's
    `--resume` fails, the step gets a note ("context not carried; handoff prompt only") that the
    Runs pane and the Chat tab show, so a fresh context is never mistaken for a resumed one. A
    failed CLI `--resume` is reported, never silently replaced by a fresh session. Do not key the
    note on `DriverCaps.resume`: for ACP rows it comes from `settings.acp.session.resume`, which
    defaults to `true` (`crates/htui-agent/src/registry.rs:165`).
  - ~~**R-49**~~: closed by MOD-37 phase 4, by admission rather than a guard. While a promoted
    step's run is `awaiting_approval`, `claim_run` refuses every other run on a
    `shared_serialized` repo of the box (rule I, `NotIsolated`), across processes. The conformance
    pin `a_promoted_shared_serialized_step_keeps_other_runs_off_its_repo` holds it. The
    in-process guard is still not re-taken, which leaves three windows open (LOW): a same-run
    command between `Promoted` and the chat bind, a second promotion of the same step (D185), and
    an isolator rebuilt during the chat.
  - ~~**R-51**~~: closed by MOD-37 phase 1. A command that queues behind a live walk publishes
    `FrameKind::Waiting`, and the pane shows "waiting for the walk" until the walk rests, fails or is
    adopted. The walk keeps its D157 lock.
  - **R-53** (re-deferred by MOD-37 phase 1; mitigated, no code change): `ItemActions` is as of the
    last `Runs` reply, so a verdict can flip before the key is pressed. The engine re-checks with
    the same admission function (D184) and the pane re-reads (D171) (drive blueprint §18). The
    pane re-requests the actions with every `Runs` read, so what is left is the interval between
    that reply and the key press, which D184 already guards.
  - **R-55**: the TUI reads `box.settings.command_limits` once per process, per server, so an edit
    does not reach its verifier until a restart or a server switch. `htui worker` re-reads them at a
    sweep with no live walk after a repo or checkout change, and a change to the limits alone at
    restart (`docs/htui-worker.md`). Nothing edits it today;
    whoever adds an editor re-reads the limits or rebuilds the verifier when no walk is live (drive
    blueprint §21.3).
  - ~~**T7's residual window**~~: closed by MOD-37 phase 1. The Chat tab sends `ChatFollow` as soon
    as the `Promoted` reply arrives, and the worker keeps a follow served before the bind for that
    bind, so the chat's frames carry the follow's address from the first one. Moving the stream
    inside the worker was ruled out: only `App::dispatch` mints a fresh seq.
  - ~~**Deadline (from ANA-27, `docs/ANA-27.md` §5.1 T4)**~~: closed by MOD-37 phase 4.
    `drive_once` runs a step session, and each fan-out candidate's, under a timer for the rest of
    `deadline_seconds`. When it fires, the session is cancelled gracefully through MOD-42's
    control, and the step settles `DeadlineElapsed` through `SettleInput::deadline_cut`, so the
    result does not depend on the clock at settle. A run cancel, even one during the cut's drain,
    still ends as `Cancelled`. Judge calls and `driver.start` are not timed; the start is bounded
    by the driver's handshake timeout.
- [ ] **MOD-36 - Weighted agent assignment across fan-out candidates** (from MOD-4 milestone 4,
  OQ-7; ANA-21 is done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`,
  `R-ORCH-7`. Milestone 4 runs every candidate of a group on the
  one agent the walk selects (rival sampling). This item spreads candidates across the eligible
  agents by weight: a weighted `AgentSelector` (the seam milestone 4 leaves per-candidate) picks each
  `fanout_index`'s agent from the walk's eligible list using ANA-21's weights, so the judge compares
  different models on the same task. Slot 0 stays `eligible[0]` and only the rival slots are
  apportioned, so the priority order stands (`docs/ANA-21.md` §5.5, §7.2). Open points to settle in
  the plan: the prompt is assembled once
  per group, but token budgeting uses one agent's estimator (`TokenEstimator::for_agent`), so mixed
  families need either the tightest budget or per-candidate trimming, which breaks "identical prompt
  across siblings" (ANA-5 `:1870`); each candidate draws on its own agent's quota; the judge must not
  learn which model wrote which candidate (position-bias control extends to model-identity bias).
  **ANA-17 note (2026-09-25, `docs/decisions/ana/ana-17.md`):** the section framing is one frame for
  every model, so the estimator is the only thing a mixed-family group can diverge on. The function is
  `TokenEstimator::for_model` (keyed on a model id), not `for_agent`, and every production
  `PromptSpec` passes `TokenEstimator::DEFAULT` today (`htui-orch/src/engine.rs:4652`, `:5374`,
  `htui/src/preview.rs:306`), so choosing it per group or per candidate is new wiring.
  **MOD-23 note (2026-09-30, `docs/decisions/mod/mod-23.md`):** ANA-21 item 8's "MOD-23's editor
  surfaces `settings.weights`" moved here: the Settings > Agents edit form carries `settings`
  untouched, and a weights field is one more entry in its field list (plan D231).
  **ANA-24 note (2026-10-01, `docs/decisions/ana/ana-24.md`):** the seed `weights.sources` registry
  takes `docs/ANA-24.md` §5: `epoch-mirrorcode` (primary) and `arena-webdev`/`arena-agent` join the
  implement axis, `epoch-eci` is narrowed to `eci_scores.csv`, `tbench-4-0` stays blocked. Each
  dynamic source needs a parser under the fetch conditions of §6 (Arena from Hugging Face only, with
  a per-fetch licence check; Epoch keyed on `benchmark_metadata.csv`, never `*_external.csv`).
- [ ] **MOD-74 - Mouse follow-ups** (from MOD-71's review, `docs/decisions/mod/mod-71.md`
  "Carried"; deferred by the maintainer, 2026-10-03). `R-TUI-1`, `R-TUI-4`. Three small gaps in the
  Runs flow view's mouse handling. (1) Review L3: an overlay, `?`, a form or a sub-tab switch turns
  capture off without reaching `RunsTab`, so a button held across the off-and-on continues the old
  pan anchor and the view jumps once. Fix: a "capture lost" hook through `App` → `Tab` →
  `DetailTab`. (2) `EnableMouseCapture` sets any-motion reporting (`?1003h`) though nothing uses
  hover; a narrower `?1000h ?1002h ?1006h` command on ANSI terminals stops the `Moved` stream at
  the source (the Windows legacy console keeps crossterm's WinAPI command). (3) A `Runs` re-read
  during a live pan shifts the viewport, and rataflow's `Panning` recomputes from its old
  `initial_viewport` on the next drag, so the view jumps once.
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees. `htui-orch`, its `Isolator` seam and `run_worker.rs` exist since MOD-4 (done, `docs/decisions/mod/mod-4.md`); the MCP half needs MOD-11.
  The named personas a `handoff` targets exist since MOD-26 (done, `docs/decisions/mod/mod-26.md`):
  `persona` registry rows, bound per phase, frozen into the run snapshot by name.
  **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T6-T7):** settle in the PRD, from OpenRig's queue
  and workflow runtime: (1) **The swarm baton is a typed exit.** An agent yields with one exit from
  a closed set the step declares (`handoff` to a named persona, `done`, `blocked` on an item,
  `failed`), validated by code. The orchestrator closes the step and creates the next step row in
  one transaction, and a per-run hop cap stops cycles. The agent proposes, the code records
  (`R-ID-6`, ANA-2 invariant 3). Each yield starts a new step with its own fresh session fed by
  documents (`R-PRM-1`). This overrides ANA-13 §3.3's "the orchestrator maintains a single shared
  context window" (`docs/ANA-13.md:59`), and the PRD must settle that conflict explicitly. (2)
  **`spawn_subagent` has a consult mode.** A read-only child, with no tree and no output document,
  returns its answer as the tool result. It is still its own `run_step` with `parent_step_id`,
  recorded and replayable (`R-HIS-1`, `R-HIS-2`), and draws on its own agent's quota (`R-AGT-7`).
  This is htui's form of OpenRig's `rig send`, without a bus (ANA-13 §3.3). (3) Open for the PRD:
  depth and breadth limits against `max_agents_per_run`; how a child fits `UNIQUE (run_id, position,
  attempt, fanout_index)`; crash recovery of a child under ANA-2 §4.9; and whether graph steps deny
  the harness's own subagent tool, whose sessions htui never sees. Cites `R-MCP-2`, which already
  lists the tool.
- [ ] **MOD-10 - Secret provider** (from ANA-7). `R-SEC-1..4`, `R-TUI-8`. `SecretProvider` trait,
  Infisical implementation, environment injection at run start, scrubber with exact-match and
  pattern masks, fail-closed persistence gate, Settings tab secret provider section. **No longer
  blocked** — MOD-2 is done (`docs/decisions/mod/mod-2.md`) and shipped the `Scrubber` seam with the
  fail-closed `MinimalScrubber` this item replaces *behind an unchanged trait*. Its call sites are
  already fail-closed on every digested byte (MOD-2 D100 as corrected by that milestone's CRITICAL);
  the one call site that was **missing** was **MOD-32**'s, now done
  (`docs/decisions/mod/mod-32.md`). **MOD-4 wired no secrets** (done,
  `docs/decisions/mod/mod-4.md`, plan D176): `htui-orch`'s `drive_once` builds every graph
  `SessionSpec` with an empty `env`, and its comment names this item as the one that fills it.
  **Relates to ANA-16** (`docs/ANA-16.md` §8, §9): open question on where secrets resolve, on the
  worker or on the server; MOD-48 owns it, with `R-SEC-2` amended only if the server resolves them.
  **Verifier boundary (MOD-62, 2026-09-29):** once this item fills the agent's `SessionSpec.env`
  with resolved secrets, that map must never reach `htui-orch/src/verify.rs`. The verifier keeps
  htui's own process environment (plan D30) and scrubs its output. A test pinning that the verify
  child does not see a resolved secret belongs to this item.
  **Run engine scrubber (MOD-61, folded in 2026-09-29):** the run engine's `MinimalScrubber` is
  built with an empty secret list (`crates/htui-worker/src/runtime.rs:764`, since MOD-41), so `scrub`'s masking half
  is inert on the run path and a `trim_record` string equal to a secret would be stored verbatim.
  When this item resolves a run's secrets, build that scrubber from the **same** map that fills the
  step's `SessionSpec.env` (`htui-orch` `drive_once`, `engine.rs:5656`), so the two cannot drift;
  the chat path already does this from `spec.env` (`agent_worker.rs:3599`) and needs the same map.
  The verifier's scrubber (`crates/htui-worker/src/runtime.rs:411`) stays pattern-only: handing it the map would put
  the resolved secrets inside `verify.rs`. A test pinning that a record string equal to a resolved
  secret is stored as `[REDACTED]` belongs to this item.
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
  **Relates to ANA-16** (`docs/ANA-16.md` §8): the MCP server must be reachable inside a container
  or on a remote box, and a stdio `McpServerSpec` must be launchable there (MOD-44; `htui worker`, MOD-41, done).
  **MOD-41 left the sink fence here** (`docs/decisions/mod/mod-41.md`, PRD D5): the step sink's
  `write_document` is still unfenced; fence it with the step's `StepFence` when this item adds the
  first production author.
  **MOD-34 left the `search_concepts` tool here** (`docs/decisions/mod/mod-34.md`): expose
  `htui_store::vector::VectorStore::search` (`R-STO-8`), scoped to the step's projects. Since MOD-50
  (`docs/decisions/mod/mod-50.md`) it also returns requirement hits; `Hit.owner` says which kind each
  hit is.
  **MOD-33 left one decision here** (`docs/decisions/mod/mod-33.md`, D277): the `box_profile` tool
  returns `BoxProfile`, and this item decides whether it honours the project's hostname switch.
- [ ] **MOD-12 - Auto mode queue runner** (from ANA-2). `R-ORCH-6`, `R-ORCH-9`, `R-ORCH-2` hard
  gates, `R-AGT-7..8` caps, `R-TUI-8`. Ready-item selection, capability filter, concurrency with
  overlap rule, queue overlay, escalation, Settings tab caps and scheduler window section. The
  `queue` action of `R-TUI-2`. Target box stored, local execution only. Design concluded in
  `docs/ANA-2.md` (§4.10, §9): `ready_items` per ANA-9 §7.4 in full, the same `claim_run`
  admission as MOD-4, batch caps as `SUM(run_step.usage)`, escalations in the queue overlay, the
  `scheduler_window` key stored but not enforced; a graph with a `gate_hard` phase is never fully
  unattended, so `FIX`/`CLEAN`/`TOOL` are the first targets. ANA-10's SQLite rewrite of
  `ready_items` (`docs/ANA-10.md` §4.8) is withdrawn with its verdict (MOD-25,
  `docs/decisions/mod/mod-25.md`): the SQLite cache is a read mirror, so `ready_items` is written for
  `PgStore` and `MemStore` only.
  **Not blocked**: MOD-4 is done (`docs/decisions/mod/mod-4.md`); `claim_run`'s admission, the
  overlap predicate, the lease, the sweep and `run_worker.rs` are there to reuse. **MOD-7 is done**
  (`docs/decisions/mod/mod-7.md`) and supplies what the capability filter needs: real
  `probed_tags`/`declared_tags`, `repo_box_path` rows by inference, and the `R-ORCH-10` refusal at
  enqueue and at claim (`Claim::MissingTags`), so auto mode reuses them rather than adding a check.
  **Relates to ANA-16** (`docs/ANA-16.md` §8): target-box selection in auto mode is MOD-43's, which
  depends on this item for its auto-mode half.
- [ ] **MOD-73 - Hand-written document versions as step inputs** (from MOD-13). `R-ENT-12`,
  `R-ORCH-2`. A hand-written new version of a step's output kind is never what the next step reads:
  `resolve_inputs` ranks this run's output, then another run's, then a hand-written one, whatever the
  version (`htui-core/src/store/mem.rs` `input_rank`, `htui-store/src/pg/read.rs` `ORDER BY CASE
  WHEN s.id IS NULL THEN 2`), and conformance `write_document_allocates_its_version` pins it. ANA-2
  §4.8 says "an edit is a new document version followed by `approved`", and MOD-13's Docs `v`
  (`docs/decisions/mod/mod-13.md`) is now how that version gets written, so the edit is shown
  everywhere but not fed to the next phase while a step-produced version of the kind exists.
  `documents_of_kinds` (prompt assembly) already takes the latest by version. Reconcile §4.2's
  ranking with §4.8, for example by letting a hand-written version newer than the run's output win,
  or by having `accept artifact` adopt it, and update the pinned case. Not blocked.
- [ ] **MOD-16 - Windows runtime verification of the agent driver** (from MOD-2). `R-AGT-1`,
  `R-NF-3`, `R-HIS-1`. **This is now the only Windows check** (TOOL-3 decided 2026-09-28,
  `docs/decisions/tool/tool-3.md`): the maintainer accepted that
  `cargo clippy --target x86_64-pc-windows-msvc` cannot run on this box, so this item carries the
  runtime half *and* the compile-level branch coverage that was lost. The line was green when
  MOD-2 wrote this entry and had caught two defects a Linux build cannot see (`c4d65ba`); it is
  green for neither crate now, `ring`'s build script being the blocker on a path that predates
  MOD-20. Every Windows-only path MOD-2 and MOD-20 wrote was never ran. Beyond the four runtime
  facts below, this item also inherits the branch coverage `ring` took away — see TOOL-3's "What
  MOD-16 inherits". The four runtime facts are:
  the job object's kill-on-close guarantee leaves no `node`/`claude` process behind (`docs/ANA-4.md`
  §11 criterion 11's Windows half); `CreateProcess` refuses a `.cmd` shim, so `${claude}` resolving
  to one must fail with a message naming the shim rather than a bare `os error 193`;
  `CREATE_NO_WINDOW` actually suppresses the console; and milestone 4's `[H-1]` buffer sealing
  behaves under Windows rename semantics, where a sealed file held open by another process makes
  the rename fail rather than silently succeed (`seal_orphaned` must warn and carry on, and
  `seal_one`'s numbered fallback must not lose a tail). Also run the Postgres suites there: the
  cache is SQLite on a different filesystem, and `append_pending`'s `OpenOptions::append` and the
  `.jsonl.open` sealing have never met a Windows file lock. Needs a Windows box with `node` and
  `claude-agent-acp` installed. Not blocked, and **MOD-2 is now done across all
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
  **MOD-7 milestone 1 added a fourth body** (`.claude/plans/mod-7-box-identity-probe.blueprint.md`
  §11; MOD-7 is done, `docs/decisions/mod/mod-7.md`), unlinted for the TOOL-3 reason and reviewed
  by eye: the `MachineGuid` fingerprint reader
  (`htui-store/src/identity.rs`, `windows-registry`); `sysinfo`'s OS, CPU and RAM facts; the GPU
  scan through an absolute-path `powershell.exe` CIM query (`htui-agent/src/box_probe/hardware.rs`);
  and two box-probe behaviours only a Windows box shows — the seed's `bash` resolving to the WSL
  launcher `%SystemRoot%\System32\bash.exe`, which boots the WSL VM and records the distro's bash
  (documented, turned off by `{"tools":{"bash":{"disabled":true}}}` in `box_probe_spec`; a code fix
  that names no tool is this item's call), and the Microsoft Store `python3` stub, which the probe now
  skips by trying each name until one prints a version (verify it holds).
  **MOD-22 and MOD-41 added two more** (`docs/decisions/mod/mod-22.md`, `docs/decisions/mod/mod-41.md`):
  whether MOD-22's loopback paste-back completes a login from a Windows box, and whether
  `htui worker`'s Windows stop path (Ctrl-Break, closing the console, a system shutdown;
  `docs/htui-worker.md`) cancels its walks and gives their leases back as it does on SIGTERM.
  **ANA-27 note on R-10 (2026-10-01, `docs/ANA-27.md` §5.1 T10):** if R-10 picks the pid-and-signal
  design, record the agent child's pid **and its start time** in `session_started`, and signal on
  adoption only when both still match. A reused pid then fails the match instead of receiving the
  signal. This does not remove the need for `unsafe` or a dependency to send the signal, and the
  death-signal alternative avoids the reused-pid hazard entirely. OpenRig treats pid plus start time
  as a process's identity in the same way (an argv token match, then a lineage fingerprint including
  `lstart` compared across two `ps` observations).
  **MOD-68 added one more** (`docs/decisions/mod/mod-68.md`, OQ-1): the `rten` embedder was checked
  only by a cross `cargo check` for Windows and macOS. Run `htui --index-items` (or a Ctrl+F search)
  there once with an empty `<cache>/htui/model`, so that the pinned fetch, the locked `.part`, the
  rename onto a file another process may hold open, and the golden tests
  (`cargo test -p htui-store --features local-embed --lib embed -- --ignored`) run on that platform.

- [ ] **MOD-70 - Follow-up command rows for engine steps** (from MOD-42, PRD Q9;
  `docs/decisions/mod/mod-42.md`). `R-AGT-1`, `R-HIS-1`. MOD-42's `run_command` table carries only
  `kind = 'cancel'`; a follow-up typed in any TUI for a step an engine walks (in process or on a
  worker, on any box) would be a second kind, applied by the lease holder like a cancel. Open first:
  the engine has no follow-up verb and ANA-2 no state that accepts one (a walk's session ends at
  `done` before the step parks, `docs/ANA-2.md:1235-1238`), and the text is typed on one box but must
  be scrubbed on the executing box (`R-SEC-3`, `R-ID-7`). Not blocked.
- [ ] **MOD-43 - Remote dispatch in the TUI** (from ANA-16, §8 item 4). `R-ORCH-11`, `R-ORCH-12`,
  `R-TUI-1`, `R-NF-3`. Target box on run start and in auto mode; a non-local target stays `queued`
  until its worker claims it; the Runs view follows `session_event` by `seq` with `LISTEN`/`NOTIFY`
  hints and a poll backstop, and shows worker liveness. Blocked on MOD-12 for auto mode only
  (MOD-42 is done: `docs/decisions/mod/mod-42.md`; its relay answers within a 1 s poll, which a
  `NOTIFY` hint would shorten).
  **ANA-27 note (2026-10-01, `docs/ANA-27.md` §5.1 T3):** the liveness the Runs view shows is
  derived from rows at read time, never reported by the agent, and "unknown" is a value. Per running
  step: *working* (a `session_event` within a window), *quiet* (none within it), *waiting on you*
  (MOD-42's open permission requests, with count and tool), *overdue* (past `deadline_seconds`;
  display only; MOD-37 phase 4 now cuts the session at the deadline), and *unknown* (the run's lease holder has not
  refreshed within the TTL). Per run: *queued, no worker* when the target box's worker has not
  checked in (`docs/htui-worker.md`, "the pane cannot tell you why"). The windows are settings, and
  each label shows the age of the evidence behind it.
- [ ] **MOD-44 - Container execution environment** (from ANA-16, §8 item 5). `R-BOX-1..3`,
  `R-AGT-5`, `R-AGT-6`, `R-AGT-9`, `R-SEC-2`, `R-MCP-1`, `R-NF-1`, `R-NF-2`. A child `box` of kind
  `container` with its own id and hostname; probe, install and auth inside the image; one container
  per session as host UID/GID; trees bind-mounted at identical paths; a launch decorator under
  `TransportBuilder` (`docker exec -i`), adapters unchanged; kill-tree is container removal; agent
  credentials on a named volume; the container never holds the DSN. Linux and macOS first (Windows
  via MOD-16). **Open question for the maintainer:** `R-NF-2` amendment (`dockerd` as an opt-in,
  per-box dependency). The MOD-7 dependency is met (done, `docs/decisions/mod/mod-7.md`: id-keyed
  registration, the box probe, capability tags); survives TUI exit through `htui worker` (MOD-41, done: `docs/decisions/mod/mod-41.md`).
  A child box's hostname never moves a digest, and the project's hostname switch covers it by
  construction (MOD-33 D275, `docs/decisions/mod/mod-33.md`).
- [ ] **MOD-46 - Live streaming via `NOTIFY` (optional)** (from ANA-16, §8 item 7). `R-HIS-1`,
  `R-NF-3`. Transient `NOTIFY` deltas under 8000 bytes between recorder flushes, droppable, superseded
  by durable `session_event` rows. Start only if 16 KiB flush bursts prove unusable; replaced by
  MOD-47's relay if phase 2 is already open. Blocked on MOD-43.
- [ ] **MOD-47 - `htui server` control plane (phase 2)** (from ANA-16, §7, §8 item 8). `R-NF-2`,
  `R-ID-2`, `R-ORCH-12`, `R-STO-1`, `R-STO-5`, `R-USR-3`, `R-SEC-1..4`, `R-ID-7`. **Trigger-gated:**
  start only when a worker runs outside the trusted network or behind NAT, team use (`R-USR-3`)
  starts, worker count exceeds the Postgres connection budget, or MOD-46 proves inadequate. The only
  worker-facing DSN holder, no durable state (`R-ID-3` stands); enrolment with server-minted box ids
  (replacing MOD-7's `box.toml`-keyed registration for such workers; MOD-7 done,
  `docs/decisions/mod/mod-7.md`),
  box-scoped auth, versioned worker protocol accepting N-1, worker-initiated WebSocket for dispatch,
  event ingest, live and permission relay; a decision on server-down behaviour. The TUI keeps
  talking to Postgres. **Open questions for the maintainer (requirement amendments):** `R-NF-2`
  (optional self-hosted server), `R-ID-2` (self-hosted control plane is not a cloud service),
  `R-ORCH-12` ("polling Postgres or the control plane"), `R-STO-1` (worker holds a box-scoped
  revocable key), `R-STO-5` (server owns migrations for workers), `R-USR-3` (roles enforced in the
  server). Not blocked: MOD-40, MOD-41 and MOD-42 are done (`docs/decisions/mod/mod-40.md`,
  `docs/decisions/mod/mod-41.md`, `docs/decisions/mod/mod-42.md`); the worker reaches the store only
  through `WorkerStore`/`WorkerHost`/`RelayStore`, which a control-plane client would implement, and
  MOD-42's `step_permission`/`run_command` rows are the payload the server relays.
- [ ] **MOD-48 - Config manager and secret distribution (phase 2)** (from ANA-16, §6.2, §8 item 9).
  `R-ID-3`, `R-AGT-9`, `R-AGT-10`, `R-SEC-1`, `R-SEC-2`. `GetManifest`/`WatchManifest` over agent
  registry, box profiles, settings, images, target build and digest; full resync on a stale cursor;
  worker-side cache; worker self-update. Targeted per-run secrets, never agent credentials
  (`R-AGT-9` unchanged). **Open question for the maintainer:** `R-SEC-2` amendment only if the server,
  not the worker, resolves project secrets. Blocked on MOD-47, MOD-10.

- [ ] **MOD-55 - Ask an agent for help while editing a template or skill** (from MOD-9, PRD gate
  2026-09-25; `docs/decisions/mod/mod-9.md`). `R-SKL-3`, `R-PRM-4`. MOD-9's editor (Skills tab, `TextArea` and `$EDITOR`,
  `.claude/prds/mod-9-skill-library-templates.prd.md` D2) gains an action that sends the body being
  edited, the role's placeholder table and the maintainer's request to a configured agent and offers
  the reply as a proposed edit, shown as a diff and saved only through the same `parse` gate. Open:
  which agent and model answer (the chat driver or a one-shot CLI call), whether the exchange is
  recorded, and how secrets in a body are scrubbed before they leave. Not blocked: MOD-9 is done (`docs/decisions/mod/mod-9.md`); the editor this extends landed with
  its milestone 1 and the Skills view with milestone 3.

- [ ] **MOD-57 - Run the external editor inside the TUI pane** (from MOD-9, `docs/decisions/mod/mod-9.md`;
  merge of MOD-7 milestone 2, maintainer-decided 2026-09-26). `R-TUI-1`, `R-TUI-7`, `R-NF-1`. Today `E`/`Ctrl+E`
  in the Templates editor (and any later user of the shared `ui::TextArea`) suspends the whole TUI
  and runs `$VISUAL`/`$EDITOR` full-screen (`crates/htui/src/editor.rs`). Offer a toggle that runs
  the editor inside the editing pane instead, with the rest of the TUI still drawn: a pseudo-terminal
  (`portable-pty`, ConPTY on Windows) feeding a VT parser (`vt100`) drawn by a ratatui widget
  (`tui-term`), keys forwarded to the child with one reserved chord to leave, resize forwarded,
  and the file read back through the same `parse` gate on exit. Works for any editor, nvim
  included. The alternative, nvim's `--embed` RPC UI (`nvim-rs`), is nvim-only and was not
  preferred. The three crates are not in `Cargo.lock`, so the plan owns that dependency decision.
  Not blocked. Its in-app widget draws through `ui::cells` (MOD-54, done:
  `docs/decisions/mod/mod-54.md`), and every other row fits through the same module (MOD-60,
  done: `docs/decisions/mod/mod-60.md`).
  **Keys (ANA-26, `R-TUI-10`):** the reserved leave chord is an action in MOD-67's catalogue, never a
  hard-coded key, and this pane is the one place `ctrl-c` is forwarded instead of quitting; the
  leave action must stay bound (`docs/ANA-26.md` §6.4).

- [ ] **MOD-67 - Configurable hotkeys: named actions, `keys.toml`, generated hints** (from ANA-26,
  `docs/ANA-26.md`, `docs/decisions/ana/ana-26.md`). `R-TUI-10`, `R-TUI-1`, `R-STO-1`, `R-NF-1`.
  Every key outside text entry becomes a named action in one compiled-in catalogue (about 100
  actions, named once per meaning: shared `list`/`pane`/`confirm`/`form`/`common` contexts, a
  narrower context overriding one for a single view), dispatched through composed context stacks
  whose resolver returns ordered candidates so a view can decline and fall through. A user
  overrides only what they change in `<config_root>/keys.toml` (per OS user and machine, never the
  store); an invalid file refuses to start with every error as `path:line`, exit status 2, kept out
  of error capture; `--keys`, `--default-keys`, `--print-keys`. `ctrl-c` is checked first and
  always quits (MOD-57's pane excepted). Every hint, key-naming message and the `?` box is
  generated; `?`/`f1` open help from every screen. Fixes two routing defects found by ANA-26:
  `ctrl-c` opens the clear confirm in Settings > Connection and Qdrant browse, and the Qdrant editor
  passes `Tab` to the tab bar. Six milestones (ANA-26 §8): M1 catalogue and resolver, M2 the file,
  M3 Settings and overlays, M4 Skills and Requirements, M5 Backlog and Chat, M6 close-out; M3-M5
  are independent but share snapshots. Not blocked.
- [ ] **MOD-69 - Waiting-on-you list across items** (from ANA-27, `docs/ANA-27.md` §5.1 T8,
  `docs/decisions/ana/ana-27.md`). `R-TUI-1`, `R-TUI-4`, `R-ORCH-2`, `R-ORCH-4`, `R-NF-3`. The Runs
  pane shows what waits on a person only for the selected item, and the top bar counts active runs,
  so with several items running a parked gate, a fan-out awaiting selection, a judge park or a
  blocked run is found by visiting items one by one. Add an overlay, opened from every screen, that
  lists every run in the active workspace waiting on a person, one row per reason (gate, selection,
  judge failure, unblock, and, once MOD-42 lands, each open permission request with its tool), read
  by one store query. `Enter` opens the item's Runs pane on that step, and the top bar gains the
  count. The list is derived at read time and keeps no state of its own; MOD-12's planned escalation
  list can reuse the query, and the opening key is a MOD-67 action. **Open question for the
  maintainer:** it needs the proposed `R-TUI-11` and the matching `R-TUI-1` top-bar line (ANA-27
  §7), not yet applied. Not blocked; the permission rows are added after MOD-42.

### Deferred backlog

- [ ] **MOD-3 - Diff tab + code explorer.** `R-LATER-1`. Later tier; needs its own ANA first.
- [ ] **MOD-5 - Issue tracker mirror.** `R-LATER-2`. `IssueSync` trait, OneDev first, downstream
  only. Later tier; needs its own ANA first.

- [ ] **MOD-8 - Legacy markdown import.** `R-LATER-3`. Map old prefixes to kinds per project,
  preserve keys, build links. Later tier; MOD-6 landed (`docs/decisions/mod/mod-6.md`, importer
  mint variant per ANA-9 §7.1 still to write). Widened by ANA-11 (`docs/ANA-11.md` §6 phase 3): also import
  `docs/REQUIREMENTS.md` into the requirement tables MOD-38 added (`docs/decisions/mod/mod-38.md`), map each `DECISIONS.md` status to
  `item.resolution` (`shipped` → `done`), write-ups to `summary` documents and each analysis doc to
  the `verdict` document, and scan body `R-` IDs once into `addresses` citations.

### Tooling findings

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 1 (ANA-25 learned weights) |
| MOD-N   | 22 (MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-16 Windows verification, MOD-27 swarm, MOD-36 weighted agent assignment, MOD-37 orchestrator hardening, MOD-70 engine follow-up, MOD-43 remote dispatch, MOD-44 container env, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-55 agent help in the editor, MOD-57 embedded editor, MOD-67 configurable hotkeys, MOD-69 waiting-on-you list, MOD-73 hand-written step inputs, MOD-74 mouse follow-ups; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 0 |
| TOOL-N  | 0 |
