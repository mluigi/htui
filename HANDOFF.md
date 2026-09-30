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

**Current status (2026-09-30):** **MOD-23 done** (`docs/decisions/mod/mod-23.md`): Settings > Agents
edits the registry. `n` creates a row, `e` edits one (transport, command and shell-word args, models,
default model, billing, enabled; `name` is create-only; a launch edit merges and never touches `env`),
and `t` switches an agent off on this box. The switch is the new `agent_box.user_off` (migration
`0008`), which no probe can undo and which refuses chat start and step promotion too. Writes are
MOD-40's compare-and-set, answered by one self-naming `AgentWritten` reply. Follow-up **MOD-66**
(per-box manual tool path).
Before it, **TOOL-7 shipped** (`docs/decisions/tool/tool-7.md`): `scripts/hr` runs
3–4 `/handoff-run` lifecycles side by side, each in its own container set (fresh clone on
`hr/<ITEM>`, private Postgres/Qdrant, no git credentials), with cross-run ID leases through
`scripts/hr-mint`; guide `docs/hr-sandbox.md`.
Before it, **MOD-39 was done** (`docs/decisions/mod/mod-39.md`): the
Requirements tab is in (tab 3, so Settings is now 4 and Chat 5), with the project → area →
requirement tree, coverage with suspect markers, the revision trail with each deciding item, and
maintainer-only create, amend and withdraw (the maintainer is the project's requirement-spec owner).
Item detail gained a **Reqs** sub-tab ("Documents" is now "Docs") to re-confirm, cite and uncite,
and the Runs pane's close-out picks the resolution, so an `open` item can now close as withdrawn,
rejected, superseded or duplicate. CLEAN-6 was folded in, and the stale `Engine::close_out` doc comments were fixed once
MOD-40 released `engine.rs`. No migration and no store change.
Earlier completions are in `DECISIONS.md`.
**Live coordinates.** The migrations are `0001_init`, `0002_agent_probe`, `0003_orchestration`,
`0004_max_agents_per_run_default`, `0005_box_identity` (MOD-7 milestone 1), `0006_requirements`
(MOD-38), `0007_skill_attachments` (MOD-9 milestone 2) and `0008_agent_box_user_off` (MOD-23; cache:
`0001`..`0004`), so **the next migration is `0009`** (cache: `0005`).
`max_agents_per_run` defaults to **8** (`0004` moves an untouched seeded `6`). Pins after MOD-7
(done, all four milestones), MOD-38, MOD-9 milestones 1 to 4, MOD-40, MOD-39 and MOD-23 (re-counted
2026-09-30): store conformance `CASES` 97, `READ_CASES` 14, `htui-orch` `CASES` 73, `GraphSource` 7
methods, `StoreRequest` 88, `StoreReply` 48, `hierarchy::REQUEST_NAMES` 13, 289 `.sqlx` files, 110
`crates/htui/tests/snapshots`,
`MIRRORED_TABLES` 21, seven Settings sections (61 of the 100 strip columns), 35 pinned commented
columns (`tests/migrations.rs`), and `run_step.trim_record` at `v: 2` with `skill_choices`.
Excerpts reach phase prompts since MOD-7 milestone 4, so a phase-prompt digest recorded before
2026-09-26 does not compare with a later one; handoff digests are unchanged.
`cargo doc --workspace --no-deps --keep-going` shows exactly six baseline errors (`htui-core`
`MIRRORED_TABLES`; `htui-store` `step_exists`, `HashEmbedder` in `embed.rs`, and three private
links MOD-38 added in `pg/write.rs`: `set_requirement_spec` to `cas_miss`, `amend_requirement` and
`withdraw_requirement` to `revise_requirement`; the count read five before 2026-09-26, but
`HashEmbedder` already failed at `98e6d2f`). `git` ≥ 2.33.0 is a runtime dependency
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
`htui-orch`, now built (MOD-4, `docs/decisions/mod/mod-4.md`). MOD-9, MOD-11, MOD-12, MOD-13 and
MOD-14 can start now (MOD-15 is done, `docs/decisions/mod/mod-15.md`; MOD-7 is done,
`docs/decisions/mod/mod-7.md`).

---

## Open items

### Analyses


- [ ] **ANA-23 - Pure-Rust local embedder to replace `ort`/`fastembed`** (maintainer-requested
  2026-09-26, during MOD-9 milestone 3). `htui-store`'s `local-embed` feature
  (`embed::FastEmbedder`, BGE-small-en-v1.5, MOD-34, `docs/decisions/mod/mod-34.md`) pulls `ort`,
  whose `ort-sys` build script downloads ONNX Runtime from `parcel.pyke.io` or needs
  `ORT_LIB_LOCATION` pointing at a native `libonnxruntime`; sandboxes that block the host can only
  build `htui-store` and `htui` with a hand-fetched library. The maintainer wants no native runtime
  fetched at build time. Compare **`candle`** (with `tokenizers` on its `fancy-regex` feature, no
  `onig`) and **`tract-onnx`** (runs the same ONNX file): offline build, binary size, embedding
  speed on the demo corpus, how and when model weights are fetched at run time, and whether the
  vectors equal the ones already stored in qdrant within a tolerance or force a re-embed. Deliver a
  verdict and the `MOD-N` that implements it.
- [ ] **ANA-24 - A licensed, effort-separated coding benchmark source for the weight map** (from
  ANA-21; ANA-21 is done, `docs/decisions/ana/ana-21.md`). `R-AGT-8`, `R-ORCH-7`.
  ANA-21's refresh is a source registry, and exactly one entry in it can be fetched today: Epoch
  (CC BY 4.0), which covers the **analysis** axis only. The implement axis has no
  dynamically-permissible source at all, so its weights are the maintainer's, entered by hand and
  dated, and an undated entry resolves to 0 rather than to a stale number. Research the gap:
  1. Find a machine-readable, redistribution-licensed source of agentic **coding** benchmark
     results. The prize is Harbor's tbench.ai: it is the official Terminal-Bench 4.0 board and the
     only source found that publishes `reasoning_effort` per row, so it would close the
     effort-separation gap that forced ANA-21 to flatten every `-medium`/`-low` string onto its
     family's `-high` value. Its results sit behind an undocumented tRPC endpoint and carry no
     licence grant; the harness repo is Apache-2.0. Establish whether the results are separately
     licensed, and by what terms.
  2. Establish whether any Terminal-Bench 4.0 results mirror exists with terms that permit fetching
     them into a product that gives model-selection guidance. Artificial Analysis's API is
     explicitly out of scope: it is internal-use only and bars exactly this use.
  3. Re-probe the sources ANA-21 recorded as blocked, since licences and endpoints move: Vals, Scale
     SEAL, Arena, SWE-bench/experiments, SWE-rebench, LiveBench, epoch-research/eci-public.
  4. Deliver a verdict naming which sources are `dynamic: true` and which `redistributable: true`,
     so ANA-21's `weights.sources` registry (§5.4) can be extended as a data edit. Blocked on
     nothing; do it whenever.
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
- [ ] **ANA-26 - Configurable hotkeys** (maintainer-requested 2026-09-29, during MOD-52;
  `docs/decisions/mod/mod-52.md`). `R-TUI-1`. The maintainer wants htui's hotkeys configurable.
  Today only a thin table is data: `Keymap::default_global` (`crates/htui/src/keymap.rs`) holds the
  global and overlay-wildcard bindings (`q`, `ctrl-c`, `Tab`, `1`..`9`, `?`, `Esc`, plus `w` from
  `register_all`), and `KeyChord::parse` already reads spec strings such as `"ctrl-c"`. Every tab,
  section and overlay key is a hard-coded `KeyCode` match in its own `on_key` (about 23 files under
  `crates/htui/src/ui`), and the hint lines spell the keys as string constants (`HINT_*`, about 36),
  so rebinding the table alone would leave those keys fixed and the hints wrong. Analyze and decide:
  1. Scope: the global and overlay table only, or every tab and section key. The latter means
     routing each `on_key` through named actions, and the shared letters (`e`, `c`, `r`, `j`/`k`
     across the Settings sections) need one action name or several.
  2. Where the bindings live and who owns them: a local file under the user's config dir, the
     store's settings, or both with an override order. Per user, per box or per workspace, and how
     R-TUI-1 and `docs/REQUIREMENTS.md` should say so (a requirement change needs the maintainer).
  3. Validation at load: duplicate chords in one scope, a printable key bound where a text field
     captures it, and whether `ctrl-c` quit may be unbound or is fixed (MOD-52 relies on every
     capturing section passing `CONTROL` chords on).
  4. How hints and the `?` help follow a rebinding, since both are generated from the table only
     for the global scope today.
  5. Prior art in other ratatui/crossterm TUIs (gitui, helix, yazi, lazygit's keybinding config).
  Deliver a verdict and the `MOD-N` that implements it.

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
  - **R-6**: nothing writes `phase_agent` for an override graph copy: `WriteStore` has no
    `phase_agent` writer at all. The `is_override` half is closed, since MOD-9 milestone 3's
    `override_graph` (`crates/htui-orch/src/graph.rs`) writes it on every copy. The plans
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
  `PromptSpec` passes `TokenEstimator::DEFAULT` today (`htui-orch/src/engine.rs:4323`, `:4945`,
  `htui/src/preview.rs:252`), so choosing it per group or per candidate is new wiring.
  **MOD-23 note (2026-09-30, `docs/decisions/mod/mod-23.md`):** ANA-21 item 8's "MOD-23's editor
  surfaces `settings.weights`" moved here: the Settings > Agents edit form carries `settings`
  untouched, and a weights field is one more entry in its field list (plan D231).
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
  **Relates to ANA-16** (`docs/ANA-16.md` §5.3, §8): a container child box has its own hostname,
  distinct from its parent's (MOD-44), so the switch and the digest split also cover child boxes.
- [ ] **MOD-28 - rataflow execution view (from ANA-12).** Add `rataflow` dependency, implement `ExecutionGraph` widget mapping `RunStep` and `SessionEvent` lists to a node graph, add view toggle to Runs tab (`R-TUI-4`), and wire mouse/keyboard events for standard run actions.
- [ ] **MOD-26 - Declarative Agent Personas (from ANA-13).** Build Markdown/Frontmatter parser in `htui-core`, discover from `~/.config/htui/agents.d/`, map to `SessionSpec` overrides (model, tools).
  **Relates to ANA-16** (`docs/ANA-16.md` §6.2, §8): personas should be registry rows rather than a
  per-box directory, so they are distributed like the rest of the config (MOD-48).
- [ ] **MOD-27 - Swarm RunKind & task MCP Tool (from ANA-13).** Add `RunKind::Swarm` to `htui-orch`, implement `spawn_subagent` MCP tool with JSON schema validation and isolated worktrees. `htui-orch`, its `Isolator` seam and `run_worker.rs` exist since MOD-4 (done, `docs/decisions/mod/mod-4.md`); the MCP half needs MOD-11.
- [ ] **MOD-65 - A panic on a blocking thread a runtime task starts leaves the terminal alone**
  (from MOD-53). `R-NF-3`, `R-TUI-8`. MOD-53 made every task the agent runtime spawns answer its
  request on a panic and polls it inside `htui_agent::excerpt::contain`, so the panic hook leaves the
  terminal alone. A panic on a `spawn_blocking` thread such a task starts is outside that window:
  the install's unpack (`htui-agent` install pipeline) and `SystemHardware::read`'s `system_facts`
  (`htui-agent/src/box_probe/hardware.rs`). The task still answers, because tokio hands the panic
  back as a `JoinError`, but the hook on the blocking thread is not vouched for and still calls
  `ratatui::restore()` under the running UI. Open the same window on those threads (a closure
  wrapped in `contain`, or a small `spawn_blocking` helper that does it). Found 2026-09-29.
- [ ] **MOD-60 - Display width in every hand-laid-out row** (from MOD-54, plan D18). `R-TUI-1`,
  `R-NF-1`. MOD-54 made `TextField` and `TextArea` count cells instead of code points through
  `crate::ui::cells`; every other place that lays text out in `char`s has the identical class of
  bug and is untouched on purpose. In order of severity: `crates/htui/src/ui/diff.rs` (the
  Templates line diff -- a CJK hunk overrunning its pane), `crates/htui/src/ui/top_bar.rs` (the
  tab bar and the clock), `crates/htui/src/ui/tabs/chat/transcript.rs` (wrap and clip), and
  `settings::wrapped` plus every row renderer under `crates/htui/src/ui/tabs/settings/` and
  `crates/htui/src/ui/tabs/backlog/detail/`. `ui::cells::cell_width` is the shared measurement to
  build on. Note the chat transcript is the one to be careful with, because a snapshot there would
  move. Also out of scope there: soft wrap, bidi, and terminals that report a CJK char as one cell.
  Not blocked; MOD-54 is done (`docs/decisions/mod/mod-54.md`). MOD-39 added two more `char`
  counters: the Requirements tab's `tree::pad`/`clip` (on a narrow pane `pad` cuts the
  ` · read-only` marker before the project name) and the Reqs sub-tab's `cut`
  (`docs/decisions/mod/mod-39.md`, "Carried").
- [ ] **MOD-59 - A write's reply names itself, so a form never stays "in flight"** (from MOD-9
  milestone 3 review, finding 3). `R-TUI-7`, `R-NF-3`. The Skills and Templates views decide that a
  save landed by finding what they sent in the re-read snapshot (`ui/tabs/skills/library.rs` `land`,
  milestone 1's D27 in `templates.rs`). If another session changes the same skill, binding or
  template between the write and the worker's re-read, or `MemStore` stamps two writes with the same
  instant, the predicate never holds: `busy` stays set and the editor, rename form or attachment
  form refuses `Esc` and `Ctrl+S` until the workspace changes. Make a write's reply identify itself
  (e.g. `StoreReply::SkillsWritten { snapshot, what }` and a templates twin) so landing no longer
  depends on content, or release the form on any reply to the write's request name. Found
  2026-09-26. MOD-39's Requirements tab lands the same way (`ui/tabs/requirements/mod.rs` `land`),
  and re-reads after a refused mint because the worker answers `Failed` when only its re-read
  failed; a reply that names its write would retire both (`docs/decisions/mod/mod-39.md`).
- [ ] **MOD-49 - Interactive path picker for repo and workspace roots** (from MOD-7). `R-BOX-4`,
  `R-TUI-8`. MOD-7 D5 infers each repo's path on a box and falls back to a typed path in a text box
  when inference fails; as built, the typed fallback is Settings > Hierarchy's `b` (MOD-7 milestone
  4 plan D115), not a box in the Boxes section. Replace the typed fallback with a popup window that
  browses the box's
  filesystem and selects a directory, reusable wherever the Settings tab asks for a path (repo paths
  in the box section, workspace roots in the hierarchy section). The chosen path goes through the
  same canonicalisation as the text box (`htui_core::root_path::canonical_root`, F-102). Listing runs
  off the UI task (`R-NF-3`). **Unblocked:** MOD-7 is done (`docs/decisions/mod/mod-7.md`).
  Raised by the maintainer at MOD-7's PRD
  gate, 2026-09-25.
- [ ] **MOD-51 - `box_probe_spec` editor in the Settings box section** (from MOD-7 milestone 2,
  OQ-18). `R-BOX-2`, `R-TUI-8`. MOD-7 milestone 1's plan (OQ-9, D15, as amended at maintainer
  review) promised milestone 2 a validated writer of the `app_setting.box_probe_spec` overlay; the
  PRD's milestone 2 row does not carry it, and the maintainer deferred it here at milestone 2's
  CONFIRM gate (2026-09-26). Milestone 2 ships only the read-only view of the effective spec. The
  writer validates through `htui_agent::box_probe` spec parsing before it stores, is a
  compare-and-set, and runs off the UI task (`R-NF-3`); the planned shape is task T6 of
  `.claude/plans/mod-7-box-settings-section.plan.md`. Unblocked: MOD-7 is done
  (`docs/decisions/mod/mod-7.md`); its milestone 2 landed
  (the Boxes section, `crates/htui/src/ui/tabs/settings/boxes.rs`, and `crate::box_settings`).
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
  `docs/decisions/mod/mod-15.md`): `htui_core::seed` writes all ten at version 1 on create. **PRD:**
  `.claude/prds/mod-9-skill-library-templates.prd.md` (2026-09-25) — everything in the Skills tab
  (templates and skills views, no Settings section), in-app `TextArea` plus `$EDITOR`, bound skills
  wired into engine and preview. Milestones 1 (templates editable), 2 (skills reach the run), 3 (skill
  writers, bindings) and 4 (SKILL.md import) have landed, the last two following ANA-22's verdict
  (`docs/decisions/ana/ana-22.md`, concluded 2026-09-25: skills attached at global, project or phase
  level with the activation on the attachment, §7 schema and import mapping, §8 phasing).
  Agent help while editing is MOD-55.
  **Phase 1 landed (`e971418`..`caacc96`, 2026-09-25):** milestone 1, templates editable — the
  `append_prompt_template` compare-and-set writer on every store (append-only, head version as the
  token), `ui::TextArea`, the `$EDITOR` handoff with the terminal suspended and SIGINT/SIGQUIT held
  off htui, `Templates`/`SaveTemplate` on the store worker, and the Skills tab's `Templates` view
  (parse-gated save with the cursor on the error, wrapped and scrollable diff between any two
  versions or against the built-in default); plan `.claude/plans/mod-9-templates-editable.plan.md`,
  blueprint `.claude/plans/mod-9-templates-editable.blueprint.md`.
  **Phase 2 landed (`7be0794`..`fd5161e`, 2026-09-26):** milestone 2, skills reach the run, widened
  by the maintainer to ANA-22's storage and activation read side. Migration `0007_skill_attachments`
  (ANA-22 §7.1: global level, `activation`/`globs`/`languages`, `skill_version.source`); one pure
  `model::skill::resolve` (most specific attachment wins, a missing winning pin never falls back)
  shared by both stores; `select` in the assembler (`always` active; `off`, `glob`→`no_path`,
  `missing_version`, `not_placed` inactive) with every choice in `trim_record.skill_choices`
  (record `v: 2`); `GraphSource::bound_skills`, so phase steps and judges get the phase's
  candidates (a judge body may place `{{skills}}`, D48, though the default judge does not: its
  replayed `{{task}}` already carries them) and handoffs none; the preview resolves the phase that
  uses the chosen template and the Prompt sub-tab lists the choices. `R-SKL-2` amended. Plan
  `.claude/plans/mod-9-skills-reach-the-run.plan.md`, blueprint
  `.claude/plans/mod-9-skills-reach-the-run.blueprint.md` (final review `rust-reviewer` APPROVE
  WITH FIXES, findings applied).
  **Phase 3 landed (`df91c82`..`e5db119`, 2026-09-26):** milestone 3, skills editable and
  attachable, **split by the maintainer** (glob *firing* is the PRD's new row 5, "Glob attachments
  fire": the F2 file set, roots for fan-out groups, changed paths, `matched`/`no_match`; unblocked
  now that MOD-7 milestone 4 runs the excerpt pass over `repo_box_path` roots). Shipped: skill names checked by the Agent Skills rule; `globset` (new
  dependency) behind `model::skill_glob` (`<repo>:` qualifiers, brace-aware lists,
  `canonical_globs`) and a seed language map (`model::skill_language`, fourteen languages); seven
  `WriteStore` methods on every store (`skills`, `skill_versions`, `skill_bindings`, `create_skill`
  with v1 in one transaction, `update_skill`, `add_skill_version`, `set_skill_binding`), each writer
  a compare-and-set with one refusal chain (`check_attachment`), `CASES` 82; the clone gap closed
  (`NewStepGraph.is_override` written, `override_graph` checks every copy before any write and
  carries the source phases' attachments; the engine's clone-gap note is gone); `crate::skills` on
  the store worker; the Skills view (library, versions, diff, `TextArea` and `$EDITOR`, token
  estimate, rename; attachments pane with a global row, winner stars, a form showing the effective
  globs and a repo picker). No migration; `.sqlx` 280. Plan
  `.claude/plans/mod-9-skills-editable.plan.md`, blueprint
  `.claude/plans/mod-9-skills-editable.blueprint.md` (F-H decided: an override clone is checked
  first, then refused). Review `rust-reviewer` APPROVE WITH FIXES: findings 1, 2, 4, 5, 7 and the
  acceptance gap (a phase `off` over a global `always`, end to end) applied (`22822ca`..`e5db119`);
  finding 3 opened as MOD-59; finding 8 accepted (documented residue in `crates/htui/src/skills.rs`).
  **Phase 4 landed (`09fd007`..`6af3f53`, 2026-09-29):** milestone 4, SKILL.md import, ported
  from PR #10 (`4abb49a`..`bbac75c`) onto the milestone 3 above; #10's own milestone 3 was not taken.
  A hand-written frontmatter reader (`model::frontmatter`, no YAML dependency, per-key issues with
  byte and line, block scalars accepted) and ANA-22 §7.3's mapping (`model::skill_import`,
  `prefill_from_source`); `StoreRequest::ImportSkills` / `StoreReply::SkillImports`
  (`skills::REQUEST_NAMES` 5 → 6) walking a file or directory on the store worker
  (`crate::skill_import`: any SKILL.md to depth 4, a rules directory's `*.md`/`*.mdc`, a hidden tool
  root's rules child only, bundled `scripts/`/`references/`/`assets/` skipped and listed, 64-file
  and 256 KiB caps asked before the read) and writing through `create_skill`, or through
  `update_skill` for a moved description and `add_skill_version` for a changed body when the name
  exists — never an attachment; the Skills
  view's `I` path form and per-file report; the attachments pane prefilling a new attachment from
  the head's `source`. Three review findings fixed in the port: a hidden tool root collected its own
  markdown and skipped its rules files, a `<name>.instructions.md` or snake-case stem was refused,
  and a name repeated in one import answered a spurious stale refusal. No migration, no
  `WriteStore` method, no `.sqlx` file, no dependency. Plan
  `.claude/plans/mod-9-skill-import.plan.md`, blueprint
  `.claude/plans/mod-9-skill-import.blueprint.md`, each headed by the port's departures.
  **Row 5 (glob attachments fire) is next, and is the last open part of MOD-9.** Carry into it: a
  `glob` attachment still records `no_path`, and an imported file's globs prefill the form but
  match nothing until row 5. Still open from milestone 3: the Skills view cannot open an editor on
  a skill with no version (review finding 6, `library.rs` `on_skill_key`); import cannot produce
  one (`create_skill` writes v1 with the row), so only a hand-written row can.
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
  built with an empty secret list (`crates/htui/src/run_worker.rs:1253`), so `scrub`'s masking half
  is inert on the run path and a `trim_record` string equal to a secret would be stored verbatim.
  When this item resolves a run's secrets, build that scrubber from the **same** map that fills the
  step's `SessionSpec.env` (`htui-orch` `drive_once`, `engine.rs:5349`), so the two cannot drift;
  the chat path already does this from `spec.env` (`agent_worker.rs:3176`) and needs the same map.
  The verifier's scrubber (`run_worker.rs:935`) stays pattern-only: handing it the map would put
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
  or on a remote box, and a stdio `McpServerSpec` must be launchable there (MOD-44, MOD-41).
  **MOD-34 left the `search_concepts` tool here** (`docs/decisions/mod/mod-34.md`): expose
  `htui_store::vector::VectorStore::search` (`R-STO-8`), scoped to the step's projects.
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
  **MOD-16** owns whether the same paste-back works from a Windows box; **MOD-7**'s box registry (done,
  `docs/decisions/mod/mod-7.md`) is where "this box is remote" would eventually be a recorded fact
  rather than a guess; it records no such fact yet. **Not
  blocked** — MOD-21 landed (`docs/decisions/mod/mod-21.md`). Found while running its live proof on
  2026-09-10.
- [ ] **MOD-66 - Per-box manual tool path editor in Settings > Agents** (from MOD-23, plan OQ-3).
  `R-AGT-6`. ANA-4 §4.6 (`docs/ANA-4.md:796-798`) describes a per-box manual entry: write
  `agent_box.path` and `probe.resolved` by hand and set `probe.status = "ready"`, protected by MOD-2
  D45/D51's `probe.source = manual` (a probe that finds nothing keeps it). In the tree that record
  never reaches a spawn: `ProbeSnapshot::recorded_launch` answers `None` for any `source` other than
  `probe` (`crates/htui-agent/src/probe.rs:1174-1177`), and the driver resolves `agent.launch`
  against `probe.tools`. So this item is the resolution rule plus a second form in the Agents section
  (beside MOD-23's create/edit pane, `docs/decisions/mod/mod-23.md`) writing `probe.tools` with
  `source: manual` for this box. MOD-23 already covers the common case, a registry row whose
  `launch.command` is a literal path, which needs no probe. **Not blocked.**
- [ ] **MOD-24 - Crash recovery of runs under the headless worker.** `R-HIS-1`, `R-ORCH-11`.
  **Rescoped by maintainer decision, 2026-09-25:** a run survives a crash through ANA-2 §4.9's
  reset-and-retry resume, not by re-hydrating the agent's context and continuing the exact step. The
  original ask (checkpoint agent memory to Postgres, re-hydrate from the last `SessionEvent`, resume
  mid-step) is **dropped**: after a crash the child process, the ACP connection and the parked
  permission responders are gone, the step may have half-mutated its tree (ANA-2 §4.9 options table),
  and `session_event` holds the transcript, not the agent's context window. What §4.9 already gives,
  as built by MOD-4 M6 (`crates/htui-orch/src/engine.rs` `sweep`/`recover_step`, conformance cases in
  `crates/htui-orch/src/conformance.rs`): a crash costs at most the interrupted step, which is reset
  to `before_hash` and retried, or marked `done` when its `after_hash` and output document exist.
  **What is left for this item:** once MOD-41 hosts run supervision, (1) a TUI exit or crash no
  longer interrupts a run the worker owns, and (2) an end-to-end test kills the `htui worker`
  process mid-step and after a step's artefacts are written, restarts it, and checks that the sweep
  resets-and-retries the first and marks the second `done`. **Not in scope:** continuing an
  interrupted step inside the agent's own session (`claude --resume <id>`, ACP `session/load` once
  MOD-37 carries it) on the unreset tree; it would need an ANA-2 §4.9 amendment, works only on the
  box that holds the agent's transcript, and can be raised again after MOD-37. Continuing across a
  worker-to-TUI re-attach of a live agent is MOD-46/MOD-47 territory, not this item's.
  Blocked on MOD-41. Relates to ANA-16 (`docs/ANA-16.md` §8), which flagged the conflict this
  decision settles.

- [ ] **MOD-41 - Headless worker (`htui worker`)** (from ANA-16, §8 item 2). `R-ORCH-12`, `R-ID-2`,
  `R-STO-1`, `R-NF-2`, `R-NF-3`. A ratatui-free entry point hosting MOD-4 M6's run supervision (lease
  refresh, sweep, one engine task per claimed run, claims only `target_box_id = self`). Reaches the
  store only through a narrow worker-store trait so a later control-plane client (MOD-47) can
  implement it. Per-worker pool size setting for the connection budget. Asks MOD-4 M6 to put
  `run_worker` in a library both binaries link. **Open questions for the maintainer (requirement
  amendments):** `R-ID-2` (as `R-ORCH-12` foresees); `R-ORCH-12` moves from later to must; `R-STO-1`
  headless DSN source (keyring `linux-native` or a systemd credential, `Cargo.toml:45-46` compiles
  only `sync-secret-service`). No blockers left: MOD-40 is done (`docs/decisions/mod/mod-40.md`),
  and the MOD-4 (M6) and MOD-7 dependencies are met (MOD-7 is done, `docs/decisions/mod/mod-7.md`,
  so the worker can register through its id-keyed `register_box`).
  **MOD-40 left these here** (`docs/decisions/mod/mod-40.md`): the worker connects with
  `PgStore::connect_headless`, which never migrates and refuses a pending schema or a build below
  `htui_target_version`. Only `append_events`, `set_step_usage` and `finish_step` are fenced by lease
  owner. `set_step_prompt`, `upsert_step_tree`, `record_commits` and the sink's output document are
  still unfenced step writes, and a holder that wakes after `done` makes the last two before its
  fenced `finish_step`. The heartbeat's self-fence reads the wall clock, so an NTP step or a suspend
  during a lease still moves it; a monotonic clock is this item's (MOD-40 blueprint F-38).
  **MOD-4 M6 landed without these asks, so they are this item's** (MOD-4 done,
  `docs/decisions/mod/mod-4.md`): `run_worker` still lives in the TUI crate
  (`crates/htui/src/run_worker.rs`, whose crate depends on `ratatui` and `crossterm`), so moving it
  into a library crate the TUI and `htui worker` both link is part of this item; no worker-store
  trait exists yet either. The MOD-4 (M6) dependency is met.
  **MOD-34 left the concepts-index sync here** (`docs/decisions/mod/mod-34.md`, `R-STO-8`):
  run `htui_store::vector_sync::Indexer::sync` as a background job; until then it is
  `htui --index-items`.
- [ ] **MOD-42 - Permission and control relay through Postgres** (from ANA-16, §8 item 3).
  `R-AGT-1`, `R-HIS-1`, `R-TUI-6`. The engine's `pump` (`record.rs:1684-1703`) cannot answer a
  parked ACP request, so engine-driven ACP steps fail on their first permission request today. The
  worker records `permission_request`, waits on a `permission_answer` row, then answers the session;
  cancel and follow-up become command rows. Answers from another box go through the relay, never
  `take_lease`. MOD-41 consumes it.
  **MOD-4 M6 landed without closing this gap** (MOD-4 done, `docs/decisions/mod/mod-4.md`): the
  engine still drives a graph step through `pump` (`crates/htui-orch/src/engine.rs:5146`, `pump` now
  at `crates/htui-agent/src/record.rs:1725-1743`) with the default `PermissionPolicy`, so engine-driven
  ACP steps still fail on their first permission request. The MOD-4 (M6) dependency is met.
- [ ] **MOD-43 - Remote dispatch in the TUI** (from ANA-16, §8 item 4). `R-ORCH-11`, `R-ORCH-12`,
  `R-TUI-1`, `R-NF-3`. Target box on run start and in auto mode; a non-local target stays `queued`
  until its worker claims it; the Runs view follows `session_event` by `seq` with `LISTEN`/`NOTIFY`
  hints and a poll backstop, and shows worker liveness. Blocked on MOD-41, MOD-42, and MOD-12 for
  auto mode.
- [ ] **MOD-44 - Container execution environment** (from ANA-16, §8 item 5). `R-BOX-1..3`,
  `R-AGT-5`, `R-AGT-6`, `R-AGT-9`, `R-SEC-2`, `R-MCP-1`, `R-NF-1`, `R-NF-2`. A child `box` of kind
  `container` with its own id and hostname; probe, install and auth inside the image; one container
  per session as host UID/GID; trees bind-mounted at identical paths; a launch decorator under
  `TransportBuilder` (`docker exec -i`), adapters unchanged; kill-tree is container removal; agent
  credentials on a named volume; the container never holds the DSN. Linux and macOS first (Windows
  via MOD-16). **Open question for the maintainer:** `R-NF-2` amendment (`dockerd` as an opt-in,
  per-box dependency). The MOD-7 dependency is met (done, `docs/decisions/mod/mod-7.md`: id-keyed
  registration, the box probe, capability tags); needs MOD-41 to survive TUI exit.
- [ ] **MOD-45 - Remote box provisioning over SSH** (from ANA-16, §8 item 6). `R-BOX-1`, `R-BOX-4`,
  `R-AGT-9`, `R-STO-1`. System `ssh` to install the matching `htui` build and a user service running
  `htui worker`; credential passed on the worker's stdin, never argv or a file; the worker
  self-registers (through MOD-7's id-keyed `register_box` and box probe, done,
  `docs/decisions/mod/mod-7.md`). Agent login via MOD-22's paste-back. SSH is not used after provisioning. Under
  phase 2 (MOD-47) it installs an enrolment token instead of a DSN. Blocked on MOD-41, MOD-22.
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
  server). Blocked on MOD-41, MOD-42 (MOD-40 is done, `docs/decisions/mod/mod-40.md`).
- [ ] **MOD-48 - Config manager and secret distribution (phase 2)** (from ANA-16, §6.2, §8 item 9).
  `R-ID-3`, `R-AGT-9`, `R-AGT-10`, `R-SEC-1`, `R-SEC-2`. `GetManifest`/`WatchManifest` over agent
  registry, box profiles, settings, images, target build and digest; full resync on a stale cursor;
  worker-side cache; worker self-update. Targeted per-run secrets, never agent credentials
  (`R-AGT-9` unchanged). **Open question for the maintainer:** `R-SEC-2` amendment only if the server,
  not the worker, resolves project secrets. Blocked on MOD-47, MOD-10.

- [ ] **MOD-55 - Ask an agent for help while editing a template or skill** (from MOD-9, PRD gate
  2026-09-25). `R-SKL-3`, `R-PRM-4`. MOD-9's editor (Skills tab, `TextArea` and `$EDITOR`,
  `.claude/prds/mod-9-skill-library-templates.prd.md` D2) gains an action that sends the body being
  edited, the role's placeholder table and the maintainer's request to a configured agent and offers
  the reply as a proposed edit, shown as a diff and saved only through the same `parse` gate. Open:
  which agent and model answer (the chat driver or a one-shot CLI call), whether the exchange is
  recorded, and how secrets in a body are scrubbed before they leave. Not blocked: MOD-9 milestone 1 (templates editable) landed 2026-09-25.

- [ ] **MOD-57 - Run the external editor inside the TUI pane** (from MOD-9, merge of MOD-7
  milestone 2, maintainer-decided 2026-09-26). `R-TUI-1`, `R-TUI-7`, `R-NF-1`. Today `E`/`Ctrl+E`
  in the Templates editor (and any later user of the shared `ui::TextArea`) suspends the whole TUI
  and runs `$VISUAL`/`$EDITOR` full-screen (`crates/htui/src/editor.rs`). Offer a toggle that runs
  the editor inside the editing pane instead, with the rest of the TUI still drawn: a pseudo-terminal
  (`portable-pty`, ConPTY on Windows) feeding a VT parser (`vt100`) drawn by a ratatui widget
  (`tui-term`), keys forwarded to the child with one reserved chord to leave, resize forwarded,
  and the file read back through the same `parse` gate on exit. Works for any editor, nvim
  included. The alternative, nvim's `--embed` RPC UI (`nvim-rs`), is nvim-only and was not
  preferred. The three crates are not in `Cargo.lock`, so the plan owns that dependency decision.
  Not blocked. Its in-app widget draws through `ui::cells` (MOD-54, done:
  `docs/decisions/mod/mod-54.md`); the rest of the display-width work is MOD-60.
- [ ] **MOD-64 - Concepts search in the TUI** (from MOD-50, 2026-09-29). `R-STO-8`, `R-TUI-2`.
  The concepts index is reachable only from the command line today (`htui --index-items`,
  `htui --search-items [--decisions]`, `crates/htui/src/concepts.rs`); inside the TUI, Qdrant is
  only a Settings section. Add a search overlay: free text, scoped to the selected project or all,
  a decisions toggle, and hits listed as `concepts::format_hit` prints them. Enter on an item or
  document hit selects that item in the Backlog; a requirement hit opens it in the
  Requirements tab (MOD-39, done: `docs/decisions/mod/mod-39.md`). Embedding the query loads the model synchronously
  (`FastEmbedder::new`, MOD-34 review), so the search runs off the UI thread, and a missing or
  unreachable Qdrant is an inline error that affects nothing else (`R-STO-8`). Whether the TUI also
  offers a re-index action, or leaves that to MOD-41's background sync, is this item's call. MOD-50
  is done (`docs/decisions/mod/mod-50.md`): hits carry an `Owner` (item or requirement), the
  resolution and the requirement state, and `concepts::DECISION_RESOLUTIONS` is the decisions set.

### Deferred backlog

- [ ] **CLEAN-4 - `LoopStop::NoProgressReview` is unreachable** (from MOD-4, risk R-9). `R-ORCH-3`.
  The review loop's no-progress predicate has two halves, and only the `after_hash` half can fire.
  `gate::reviews_are_identical` reads through the latest-only `documents_of_kinds`, so it can never
  hold two review documents to compare, and `LoopStop::NoProgressReview` has been unreachable since
  milestone 2. No test reaches it; only its `Display` is tested. Fix it through `documents()` heads,
  in a change that first pins which stop reason each shipped loop case reaches, because the fix can
  change that. Source: `.claude/plans/mod-4-orch-fanout.blueprint.md` F-B and §11, carried unchanged
  through milestones 5 and 6 (`docs/decisions/mod/mod-4.md`, "Carried").
- [ ] **CLEAN-7 - Leftovers of the removed offline chat buffer** (from MOD-39, 2026-09-29).
  `R-NF-3`. MOD-25 refused offline chats and the buffer machinery was deleted later; MOD-39 fixed
  the comments (`docs/decisions/mod/mod-39.md`), which left code and text: the Settings rebuild
  confirm still says "the pending/ buffer" survives (`ui/tabs/settings/connection.rs` ~140 and
  ~996, asserted in `tests/connection.rs` and the `connection__confirm` snapshot);
  `RefreshSettings::this_user` (`htui-store/src/cache/refresh.rs`) is set and never read;
  `agent_worker.rs` `quota_latch_for` always answers `Some` and it and `project_caps_for` take an
  unused `_writer`; `writer_label` still rides `StoreReply::ChatAccepted` and `ChatArgs` though the
  Chat tab ignores it; and test names and messages still say upload or buffered
  (`mem.rs` `a_chat_run_mints_the_two_rows_the_offline_upload_would`, `recorder.rs` ~1531,
  `agent_worker.rs` ~5828/5976/7170, `chat_offline.rs`, `tests/cache.rs` `pending_event`).
  No behaviour change.
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
| ANA-N   | 4 (ANA-23 pure-Rust embedder, ANA-24 licensed coding benchmark source, ANA-25 learned weights, ANA-26 configurable hotkeys) |
| MOD-N   | 35 (MOD-9 skills, MOD-10 secrets, MOD-11 MCP, MOD-12 auto mode, MOD-13 editing, MOD-14 graph, MOD-16 Windows verification, MOD-22 loopback paste-back, MOD-24 worker crash recovery, MOD-26 personas, MOD-27 swarm, MOD-28 rataflow, MOD-33 hostname out of the digest, MOD-36 weighted agent assignment, MOD-37 orchestrator hardening, MOD-41 headless worker, MOD-42 permission relay, MOD-43 remote dispatch, MOD-44 container env, MOD-45 SSH provisioning, MOD-46 NOTIFY streaming, MOD-47 control plane, MOD-48 config manager, MOD-49 path picker, MOD-51 probe spec editor, MOD-59 write replies name themselves, MOD-60 display width, MOD-65 blocking-thread panics, MOD-55 agent help in the editor, MOD-57 embedded editor, MOD-64 TUI concepts search, MOD-66 per-box tool path editor; deferred MOD-3 diff, MOD-5 tracker, MOD-8 import) |
| CLEAN-N | 2 (CLEAN-4 unreachable `NoProgressReview`, CLEAN-7 offline-buffer leftovers)            |
| TOOL-N  | 0 |
