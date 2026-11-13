# Plan: MOD-164 - Automatic memory capture at run end (milestones 1 and 2)

**Source PRD**: `.claude/prds/mod-164-automatic-memory-capture.prd.md`
**Selected Milestones**: 1 (run write path) and 2 (extraction turn), planned together (maintainer, 2026-11-13)
**Complexity**: Large
**Routing**: PRD (C2, C3, C4), ultracode for implement (lane shape), `/handoff-run` 2026-11-13.
**Status**: fact-checked (23 claims, amended; see "Verified claims"); awaiting CONFIRM.

## Summary
A run that saw a failure or a retry gets one more step before it ends. The step is a `capture` stage row at
position -4. It runs the light model over a labelled evidence pack built from the run's own rows. The model
answers with JSON through `document_write`. Deterministic code then validates that JSON: evidence quotes must
appear in the pack, everything is scrubbed, and a run writes at most 3 entries. Valid entries are written
through one new store verb, which stamps provenance and supersedes only run-written entries. `finish_run`
counts each entry's usage exactly once. The capture step can never change the run's outcome.

## Shared decisions (every brief carries this section)

**D1 - Migration `0031_memory_capture.sql` (Postgres only).**
No cache migration is needed. Memory is not mirrored (`cache/read.rs:1316`). The mirror has no CHECKs
(`cache_migrations/0010_divided_fanout.sql:3`). `refresh_run_step` copies an explicit column list, and 0031
adds no `run_step` column. The migration moves `PgStore::schema_version()` to 31, which forces one mirror
rebuild per box (expected).

`memory_entry` gains these columns:
- `source TEXT NOT NULL DEFAULT 'person' CHECK (source IN ('person','run'))`.
- Four nullable references: `run_id UUID REFERENCES run(id) ON DELETE SET NULL`,
  `step_id UUID REFERENCES run_step(id) ON DELETE SET NULL`, `item_id UUID REFERENCES item(id) ON DELETE SET NULL`,
  and `agent_id UUID REFERENCES agent(id) ON DELETE SET NULL`.
- `model TEXT`, `commit_hash TEXT`, and `evidence JSONB`.
- Three counters, `shown_steps`, `shown_done` and `shown_failed`, each `INTEGER NOT NULL DEFAULT 0 CHECK (>= 0)`.

It also gains a constraint and an index:
- `CONSTRAINT chk_memory_entry_person_has_no_run CHECK (source = 'run' OR (run_id IS NULL AND step_id IS NULL AND agent_id IS NULL AND model IS NULL AND commit_hash IS NULL AND evidence IS NULL))`.
- `CREATE INDEX idx_memory_entry_step ON memory_entry(step_id) WHERE step_id IS NOT NULL`, for the write cap's count.

Comments:
- Add a `COMMENT ON COLUMN` for every new column. A new `MOD164_COLUMN_COMMENTS` constant in
  `tests/migrations.rs` is chained into all three chains of `the_ana_column_comments_are_present_and_verbatim`,
  which asserts "exactly the listed commented columns".
- A new `COMMENT ON TABLE memory_entry` says entries are written by a person or by a run's capture step
  (MOD-164). `MOD163_TABLE_COMMENTS` compares against the live database, so it moves to the new text.
- Restate `COMMENT ON COLUMN run_step.node_key` to name `capture` as well. `MOD139_COLUMN_COMMENTS` moves.
- 0022's and 0029's SQL is never edited.

`run_step`: drop and re-add `chk_run_step_stage`:
`(role = 'stage') = (position < 0) AND (role <> 'stage' OR (position BETWEEN -4 AND -1 AND node_key IS NOT NULL AND node_key IN ('capture','pick','prepare','collect')))`.

There is no new table. `TABLES` stays 52, and `pg_criteria.rs`'s cascade list does not move.

**D1a - Migration count pins** (all move from 30 to 31, prose included):
- `tests/connect.rs`: :140, :157, :242.
- `tests/migrations.rs`:
  - the versions vector `1..=30` at ~:120, plus its prose at :134;
  - `Pending(30)` at :4168 and :4269;
  - `MigrationsPending(30)` at :4278 and :4301;
  - the `_sqlx_migrations` count at :4453 ("thirty embedded migrations").

**D2 - Model (`htui-core/src/model/memory.rs`).**
- `MemorySource` gains `Run => "run"`, whose `trust()` is `"model-written"`. The pin
  `memory_source_is_person_and_person_written` moves to `[Person, Run]`.
- `MemoryEntry` gains four fields:
  - `source: MemorySource`;
  - `provenance: Option<RunProvenance>`, which is `Some` iff `source == Run`. Its type is
    `RunProvenance { run_id: Option<RunId>, step_id: Option<StepId>, item_id: Option<ItemId>, agent_id: Option<AgentId>, model: Option<String>, commit: Option<String> }`.
    Every field is an `Option` because each FK is `SET NULL`.
  - `evidence: Option<MemoryEvidence>`;
  - `usage: MemoryUsage { shown: i32, done: i32, failed: i32 }`.
- `MemoryEvidence { citations: Vec<MemoryCitation> }` and `MemoryCitation { step_id: StepId, quote: String }`
  derive `Serialize` and `Deserialize`, and are stored as the `evidence` JSONB.
- `pub const MEMORY_RUN_WRITE_CAP: u32 = 3;`
- **Postgres decode.** The four memory reads use `sqlx::query_as!(MemoryEntry, …)`: `pg/read.rs:1119`
  `memory_entries`, and in `pg/write.rs` `memory_cas_head` :758, `apply_memory_edit` :829 and
  `insert_memory_entry` :887. They move to a flat `MemoryEntryRow` (in `pg/rows.rs`) with
  `impl From<MemoryEntryRow> for MemoryEntry`. `cargo sqlx prepare` regenerates their `.sqlx` entries.
- There are only three other `MemoryEntry { … }` literals: `htui-core/src/prompt/fixtures.rs:794`,
  `htui-core/src/store/mem.rs:9217` and `htui/tests/settings_memory.rs:41`.

**D3 - Capture stage key.**
- In `htui-core/src/model/run.rs`:
  - `STAGE_KEYS: [&str; 4] = ["capture", "pick", "prepare", "collect"]`;
  - `pub const CAPTURE_KEY: &str = "capture";` and `pub const CAPTURE_POSITION: i32 = -4;`;
  - the doc comments at :1102-1111 and on `RunStep.node_key` (:385).
- No code maps a key to a position. `run_step_refusal` (`store/traits.rs:2899`) checks a range and a key set
  separately: the range becomes `-4..=-1`, and both refusal sentences change. Update the doc comments at
  traits.rs:1713 and :2897.
- As in the database CHECK, -4 with `pick` is accepted. Nothing binds a key to its position.
- Three tests that today assert -4 is refused must move in T1:
  - `traits.rs:4792` `run_step_refusal_mirrors_the_0028_checks`;
  - `htui-core/src/store/conformance.rs:29882` `create_step_reserves_negative_positions_for_stages`, together with
    the sentences pinned by `step_refused_exactly`;
  - `htui-store/tests/migrations.rs:~6829` `the_0029_step_role_checks_refuse_what_they_name`, which is
    `#[cfg(feature = "demo")]`.
  Each now uses -5 for "outside the reserved range".

**D4 - Write verb.**
- `WriteStore::write_run_memory(&self, fence: StepFence, step: StepId, new: NewRunMemory) -> Result<RunMemoryWrite>`,
  with a `WorkerStore` twin.
- Implementors: `MemStore` (`mem.rs` and core `worker.rs`), `PgStore` (`pg/write.rs` and store `worker.rs`),
  `Writer` (store `writer.rs` and `worker.rs`), `UsageSpy` (`htui-agent/src/conformance.rs`) and `SpyStore`
  (`htui-agent/tests/recorder.rs`). `Backend` implements no `WriteStore`.
- `NewRunMemory { id: MemoryEntryId, kind: MemoryKind, subject: MemorySubject, body: MemoryBody, evidence: MemoryEvidence, commit: Option<String> }`.
- `enum RunMemoryWrite { Created(MemoryEntry), Superseded { old: MemoryEntry, new: MemoryEntry }, PersonHolds { existing: MemoryEntryId }, CapReached { cap: u32 } }`.

It runs in one transaction:
1. `step_scope(…, step, fence)` (`pg/write.rs:330`; MemStore `mem.rs:8265`). It does not return role or agent,
   so the verb also reads the step row (`role`, `node_key`, `agent_id`, `model`) and the run row (`item_id`,
   `started_by`, `lease_box_id`).
2. The step must be `role = stage` and `node_key = capture`. Otherwise refuse with
   `Constraint("memory_needs_a_capture_step")`.
3. If at least `MEMORY_RUN_WRITE_CAP` entries already have `step_id = step`, answer `CapReached`.
4. Look up a live entry with the same folded subject in the run's project:
   - none: insert the new entry;
   - `source = person`: answer `PersonHolds` and write nothing;
   - `source = run`: supersede it the way `supersede_memory_entry` does. The old entry becomes `superseded` with
     `superseded_by` set and a `superseded` revision; then insert the new one.
5. Stamp provenance from the rows, never from `new`: `run_id`, `step_id`, `item_id`, `agent_id`, `model`,
   `created_by = run.started_by`, and the revision's `box_id = run.lease_box_id`. `commit` comes from `new`; the
   engine code fills it (D11), never the model. Revision 1 has reason `created`.

**D5 - Usage counters in `finish_run`, exactly once.**
- `finish_run` refuses a terminal run through `legal_move`: `mem.rs:8134` and `pg/write.rs:1797-1826`
  (`FOR UPDATE`). It therefore runs once per run.
- Right after `legal_move`, in the same transaction (`finish_run_on` in Postgres, `State::finish_run` in MemStore):
  - For every step of the run whose raw `trim_record` JSON has `memory.shown[]`, take each listed `id` that
    still exists.
  - Add 1 to its `shown_steps`.
  - Add 1 to `shown_done` if the step's status is `done`, or to `shown_failed` if it is `failed`.
- `trim_record` is a raw `Value` in the stores. `MemoryAudit` is `Serialize`-only, and its shape is
  `{"memory":{"shown":[{"id","version"}],"total"}}`. Postgres uses `jsonb_array_elements`.
- `close_batch` also calls `finish_run_on`, on queued runs with no steps, so it tallies nothing.
- Chat runs end through `finish_chat_run`/`fail_run` and are not counted: chat prompts carry no memory section.
- Nothing else writes the counters, and nothing reads them yet. Demotion is MOD-166/169's.

**D6 - Prompt source.** `prompt::memory::select` freezes `entry.source` instead of the constant `Person`.
The cap, order and rendering are unchanged, so run-written entries share MOD-163's 1,500-token cap and its
newest-first order. There is no `GraphSnapshot::V` bump: migration 0031 already makes older builds refuse the
schema.

**D7 - Light-model setting.** `SettingKey::LightModel`, key `light_model`.
- **Placement.** It takes index 9 in byte order, so `MaxSkillTokens`..`TokenBudget` shift by one.
- **Kind.** A new `SettingKind::AgentModel`, `Rungs::APP` only.
- **Unset.** The default is `Value::Null`, meaning "not configured", and `Defaults::integer` gives 0.
  `as_rows` emits `("light_model", Null)`, which is harmless because only tests read it. Both "unseeded" filters
  add it: `settings.rs:982-983` and `tests/prompt_settings.rs:657`.
- **Value.** `{"agent_id":"<uuid>","model":"<1..=200 chars>"}` with no other fields. `validate` checks the shape
  only.
- **Resolver.** `pub fn resolve_light_model(app: &BTreeMap<String, Value>) -> Option<LightModel>`, with
  `LightModel { agent_id: AgentId, model: String }`. A malformed value resolves to `None`. Re-export both from
  `prompt/mod.rs:49`.
- **Pins** (fourteen keys become fifteen, including the prose; `memory_capture` moves to index 11):
  - in `settings.rs`:
    - `ALL: [Self; 14]` (:214) and `SPECS: [SettingSpec; 14]` (:377);
    - `rows.len()` 12 (:976-979), `app.len()` 12 (:1290-1292) and `count()` 12 (:1294);
    - `SettingKey::ALL[10]` (:1185), which becomes `ALL[11]`;
    - the "fourteen" prose at :169, :253 and :1415;
  - in `tests/prompt_settings.rs`: the counts at :165 and :646, plus the "twelve" prose.
- **Exhaustive matches to extend:** `value_of`, `integer` and `validate` in `settings.rs`; `effective` (~:1114),
  `submit` (:637), `unit` (:1213) and `range_text` (:1225) in `htui/src/ui/tabs/settings/prompt.rs`.
- **Settings › Prompt row.** A "Light model" row shows `<agent name> · <model>`, or `not set`.
  - The snapshot `SettingsSnapshot` (`htui/src/prompt_settings.rs`) has no agents. It gains
    `agents: Vec<AgentSummary>`, read with `backend.agents()` inside the same snapshot read.
  - The edit act on the row cycles through `(agent, model)` pairs, then `not set`. The pairs come from enabled
    agents only; each contributes its `models`, or its `default_model` when `models` is empty. An agent with
    neither is left out, which differs from `node_form::agent_pairs`.
  - Choosing `not set` calls `clear_setting`, which deletes the row.
  - No new keys, so `docs/htui-keys.md` does not move. MOD-123 later moves the row under Settings › Workflows.
- **No model-membership check**, as for the judge: ACP agents list no models until their first handshake.

**D8 - Freeze at run start.**
- `GraphSnapshot` gains `#[serde(default)] capture: Option<SnapshotCapture>`, next to MOD-163's `memory`.
  - `SnapshotCapture { model: Option<SnapshotCaptureModel> }` and
    `SnapshotCaptureModel { agent_id: AgentId, agent_name: String, model: String }`.
  - `None` means capture is off: the switch is off, or the snapshot is from before this change.
  - It is outside `topology_v2`, so there is no `V` bump.
- `graph::resolve_or_refused` fills it:
  - `Some` iff `resolve_memory_capture(Some(&project.settings), app)`;
  - `model` comes from `resolve_light_model(app)` plus `source.agent(id)`. An agent that is missing or disabled
    gives `None`.
- Every `GraphSnapshot { … }` literal gains `capture: None,`. These are the same 18 sites that carry
  `memory: None,`: htui-agent tests `relay.rs`; htui-core `fixtures.rs`, `model/run.rs`, `prompt/fixtures.rs` (×4),
  `store/conformance.rs` and `store/mem.rs`; htui-mcp tests `tools_backlog.rs`; htui-orch `engine.rs`,
  `graph.rs` and `promote.rs` (×2); htui-store tests `cache.rs` (×2) and `pg_criteria.rs`; htui tests
  `backlog.rs`. Plus any test-only literal the compile finds.

**D9 - Trigger (pure).** `capture_trigger(steps: &[RunStep]) -> Option<CaptureReason>`.
- It looks only at rows that are not stage rows and have `fanout_index >= 0`, so judges are excluded.
- It fires when any such row has `status == Failed`, `attempt > 1`, `verify_outcome == Some(Fail)`, or a
  `gate_outcome` of `Rejected` or `Retried`.

**D10 - Evidence pack, prompt and output (pure, new module `htui-core/src/capture.rs`).**

`EvidencePack::build(steps: &[RunStep], outputs: &BTreeMap<StepId, String>) -> EvidencePack`:
- One block per notable step, labelled `S1..Sn` in walk order (position, attempt).
- A step is notable if it meets D9, or if it is the passing attempt right after a failed attempt at the same
  position (the "win").
- A block holds the phase name, attempt, status, `exit_code`, verify outcome, gate outcome and `gate_note`, and
  the tail of the step's latest `command_run.output`.
- The pack is capped at `CAPTURE_INPUT_TOKEN_CAP = 6_000` (the `chars-v2` estimator). Output tails are cut first,
  then whole oldest blocks.
- It maps each label to its `StepId`.

`capture_prompt(pack: &EvidencePack) -> AssembledPrompt`:
- This is an internal fixed text, not a user template. Build it with a constructor that gives the same digest
  and sections shape `assemble` produces, because `open_recorder` takes an `&AssembledPrompt` (`prompt/mod.rs:505`).
- The text is the pack plus these rules:
  - at most 3 entries;
  - kinds `fact`, `decision`, `gotcha` and `lesson`;
  - subject at most 120 characters, body at most 2000;
  - every entry cites at least one block with a verbatim quote of at least 12 characters;
  - write lasting project knowledge, not a story of the run;
  - zero entries is a valid answer;
  - call `document_write` once with `{"entries":[{"kind","subject","body","evidence":[{"block","quote"}]}]}`.

`parse_capture(body: &str, pack: &EvidencePack, scrubber: &dyn Scrubber) -> Result<CaptureOutput, CaptureRefusal>`,
where `CaptureOutput { kept: Vec<CapturedEntry>, dropped: Vec<(usize, DropReason)> }`:
- The body is parsed with serde `deny_unknown_fields`. A body estimated over `CAPTURE_OUTPUT_TOKEN_CAP = 2_000`,
  or one that does not parse, is refused whole.
- Each entry is dropped with a reason unless all of these hold:
  - its kind parses;
  - `MemorySubject::new` and `MemoryBody::new` accept the text (scrubbed, fail-closed);
  - its evidence is non-empty;
  - every citation names a block in the pack, and the trimmed quote (at least 12 chars) is a substring of that
    block's text.
- Quotes are scrubbed, and block labels are mapped to `StepId`.
- Entries past the third are dropped as `OverCap`.
- `CapturedEntry` maps onto `NewRunMemory` minus `id` and `commit`.

**D11 - Engine run end (`htui-orch`).**

`Engine::end_run(&self, run, to, failure, at)` replaces the 16 non-test `finish_run` calls in `htui-orch/src`:
- engine.rs: 1629, 2696, 4004, 4055, 4150, 4985, 5766, 6076, 6098, 6706, 6805, 7290, 8120 and 8388;
- gate.rs: 880 and 1151.

Ways an item run can end without capture, and why each is fine:
- Queued runs end through `cancel_queued` (`transition_run`, engine.rs:2657), `close_batch` and `claim_run`. They
  have no steps, so no capture is owed.
- Chat runs are out of scope.

When `to != Cancelled`, `snapshot.capture` is `Some`, and the run has an item, `end_run` calls `self.capture(run)`
and then `finish_run`.

Code layout: `capture` lives in `src/engine/capture.rs`, declared `mod capture;` inside `engine.rs`. That makes it
a child module that can reach the engine's private items: `drive_once` (:9160), `open_recorder` (:8967),
`output_of` (:9709), `step_started`, `move_step` and `parts`.

`capture` returns `Err` only for fence, lease or cancel errors. Every other failure becomes a step outcome.

`capture(run)`:
1. **At most once.** If a capture row exists and is `running` (left by a crash), settle it with
   `interrupt_step(step, "INTERRUPTED: …")` (failed plus a note) and do not re-run it. A capture row in any other
   state means return now.
2. **No trigger.** No D9 trigger means return: no row, no note.
3. **No light model.** `capture.model` is `None` means: `create_step` the capture row, `transition_step`
   Pending→Running, then `pass_step(Lease, Some("skipped: no light model configured"))`. The row ends `done`
   with `gate_outcome = skipped` and that note. No verb settles `cancelled` with a reason; `pass_step` is the
   existing one. No item note is written.
4. **Run the turn.**
   - `create_step` a row with role `stage`, position -4, `node_key` and `phase_name` `capture`, and the frozen
     agent and model. Move it Pending→Running and call `step_started`.
   - `isolator.prepare(run, step, &[], Isolation::Local, None)`.
   - Build a synthetic `SnapshotPhase` named `capture`: `output_kind = "memory_capture"` (so `open_tools` offers
     `document_write`), one candidate with the frozen agent and model, and gate never.
   - Build the pack from `run_steps` plus `command_runs` for each notable step, then `capture_prompt`.
   - `set_step_prompt`, then `open_recorder` (the run spend cap applies).
   - `drive_once(…, deadline = Some(now + CAPTURE_DEADLINE))`. `CAPTURE_DEADLINE` is a new constant of 10 minutes;
     the judge is untimed.
   - `sink.after_done(…)`, which is how the fake writes its document.
   - `output_of(item, &capture_phase, step)`.
5. **Write.** Run `parse_capture`. For each kept entry call `write_run_memory`, with `commit` set to the newest
   `after_hash` in the run's `step_commits`, or `None`.
6. **Settle.**
   - Success: `transition_step` Running→Done, plus the item note
     `memory capture: wrote N (superseded M), kept a person's entry K, dropped D (reasons)`.
   - Any failure (session failed, cap breach, timeout, no document, refused output): `interrupt_step(step, reason)`
     plus the item note.
   - The run's own status never changes.

**Cancel during capture.** When `drive_once` answers `Cancelled`, `end_run` does not see it; the cancel path ends
the run. `cancel_unsettled` (engine.rs:4469) today skips every stage row. It is narrowed to skip
`pick`/`prepare`/`collect` only, so a `capture` row left `pending` or `running` is cancelled with its sibling rows.

**Recovery** needs no new arm. `recover_run` still skips stage rows (:3443). A crashed run is still `running`, so
`recover_run` returns `Next::Walk`, and `run_to_rest` reaches `Pass::Finished`, or `settle_failed` reaches
`end_run`. Step 1 then interrupts the stale row.

**D12 - Pane.** `ui/tabs/settings/memory.rs::row_line` (:1395) adds a source column (`person` or `run`) after the
state. The `settings_memory` snapshots move.

**Conventions** (from MOD-163):
- Tests are written first.
- Refusals are `StoreError::Constraint` with a stable code word.
- Store behaviour is pinned by the shared conformance cases: add the name to `CASES` and an arm in
  `run_case_inline`. Orch cases go in a group function chained in `case()`.
- An implementer runs only its task's check, plus clippy and rustfmt on the crates it touched. Never the
  workspace gate.
- Features:
  - `htui` needs `--features testkit`;
  - every other crate needs `--features test-support`, which enables `demo`;
  - without them, integration tests run zero tests and still pass.
- New or changed `sqlx` SQL needs `cargo sqlx prepare` against a migrated scratch database (`docs/hr-sandbox.md`).
- Run DB tests with `--test-threads=1`.
- Commit after each green sub-step.

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Fenced step write | `htui-store/src/pg/write.rs:9613` `write_step_document`, `step_scope` :330 | `FOR SHARE` step+run; refusal code words |
| Supersede | `pg/write.rs` `supersede_memory_entry` (~:10361); `mem.rs:9355` | One tx, deferred FK, `superseded` revision |
| Provenance columns | `0001_init.sql` `document.produced_by_step_id`, `item_note.via_step_id` | Nullable step FK, `SET NULL` |
| One-off model turn | `htui-orch/src/engine.rs` `run_judge` :7463, `judge_sessions` :7938, `judge_calls` :7994 | Row, prompt, recorder, `drive_once`, `after_done`, `output_of` |
| Settings key | `prompt/settings.rs` `MemoryCapture` row, `resolve_compete_fan_out` | `SPECS` row, resolver, pins |
| Agent/model pairs | `ui/graph_editor/node_form.rs:1350` `agent_pairs` | Pairs of agent and model |
| Snapshot freeze | `htui-orch/src/graph.rs:460` `frozen_memory` | Optional `GraphSnapshot` field, written once at resolve |
| Conformance | `htui-core/src/store/conformance.rs` memory cases; `htui-orch/src/conformance.rs` `memory_case` :1392, `stop_case` | Name in `CASES` plus a group arm |

## Files to Change (by task)
| Task | Files |
|---|---|
| T1 | `htui-store/migrations/0031_memory_capture.sql` (new); `htui-core/src/model/memory.rs`; `htui-core/src/model/run.rs` (stage keys, docs); `htui-core/src/store/traits.rs` (`run_step_refusal`, docs, its tests); `htui-core/src/store/mem.rs` (row shape, person defaults); `htui-core/src/store/conformance.rs` (only `create_step_reserves_negative_positions_for_stages` and its sentences); `htui-core/src/prompt/fixtures.rs` (the `MemoryEntry` literal); `htui-store/src/pg/rows.rs`, `pg/read.rs`, `pg/write.rs` (the four memory decode sites, insert columns); `htui-store/tests/migrations.rs`, `tests/connect.rs`; `htui-store/.sqlx/*`; `htui/tests/settings_memory.rs` (the `MemoryEntry` literal only) |
| T2 | `htui-core/src/store/traits.rs` (verb); `htui-core/src/store/worker.rs`; `htui-core/src/store/mem.rs`; `htui-core/src/store/conformance.rs`; `htui-core/tests/mem_store.rs`; `htui-store/src/pg/write.rs`; `htui-store/src/worker.rs`; `htui-store/src/writer.rs`; `htui-store/tests/pg_conformance.rs`; `htui-agent/src/conformance.rs`; `htui-agent/tests/recorder.rs`; `htui-store/.sqlx/*` |
| T3 | `htui-core/src/prompt/settings.rs`; `htui-core/src/prompt/mod.rs` (re-export); `htui/src/ui/tabs/settings/prompt.rs`; `htui/src/prompt_settings.rs`; `htui/tests/prompt_settings.rs` and its snapshots |
| T4 | `htui-core/src/capture.rs` (new); `htui-core/src/lib.rs` (mod line); `htui-core/tests/capture.rs` (new) |
| T5 | `htui-core/src/prompt/memory.rs`; `htui-core/tests/prompt_memory.rs`; `htui/src/ui/tabs/settings/memory.rs`; `htui/tests/settings_memory.rs` and its snapshots |
| T6 | `htui-core/src/model/run.rs` (`GraphSnapshot.capture`, types); the 18 `GraphSnapshot` literal sites (D8); `htui-orch/src/graph.rs`; `htui-orch/src/engine.rs`; `htui-orch/src/engine/capture.rs` (new); `htui-orch/src/gate.rs`; `htui-orch/src/conformance.rs`; `htui-orch/tests/fake_conformance.rs`; existing orch and engine tests whose step lists gain a capture row (D11 step 3) |

**Independence.** T1, T2, T4, T5 and T6 overlap: they share `run.rs`, `traits.rs`, `mem.rs`,
`store/conformance.rs` and `settings_memory.rs`, and T4 and T5 need T1's types. They run serially. T3's files
(`prompt/settings.rs`, `prompt/mod.rs`, `htui/src/ui/tabs/settings/prompt.rs`, `htui/src/prompt_settings.rs`,
`htui/tests/prompt_settings.rs`) appear in no other task's list (claim O11), so T3 runs in a parallel lane. T6
reads `resolve_light_model`, so lane B merges before T6.

## Execution shape
- **Lane A** (main checkout, serial): T1 → T2 → T4 → T5.
- **Lane B** (worktree on branch `mod-164-t3`, at the same time as lane A): T3.
- **Then** merge B into `hr/MOD-164` and run **lane C**: T6.
- **Verifiers**: T1 and T2, the foundation tasks. The rest go to the gate and the reviewer.
- **Gate** (main thread): `bash scripts/gate.sh` after A and B merge, and again after T6. The sandbox's live Qdrant
  tests are owed on the host.

## Tasks

### T1 - Schema, model and capture stage key (foundation)
- **Action**
  - D1 and D1a: the migration, comments and pins.
  - D2: the types; the flat `MemoryEntryRow` at the four `query_as!` sites; existing inserts write
    `source = 'person'`.
  - D3: the key, range, sentences, docs and the three moved tests.
  - MemStore mirrors the Postgres changes.
  - Add the test `the_0031_checks_refuse_what_they_name` (under `demo`): a person row with `run_id` is refused;
    -4/`capture` and -4/`pick` are accepted; -5 is refused.
- **Tests first**
  - `memory.rs`: `memory_source_run_is_model_written`; the `MemoryEvidence` serde round trip.
  - `run.rs`: `capture_is_a_stage_key_at_minus_four`.
  - `traits.rs`: -4/`capture` is accepted and -5 is refused.
  - The moved tests listed in D3; the migration and comment pins.
- **Check**
  - `cargo test -p htui-core --features test-support -- --test-threads=1` (this runs `tests/mem_store.rs` too)
  - `cargo test -p htui-store --features test-support --test migrations --test connect --test pg_conformance -- --test-threads=1`
  - `cargo test -p htui --features testkit --test settings_memory --no-run`
  - `cargo clippy -p htui-core -p htui-store --features test-support -- -D warnings`
  - `cargo fmt --check`
  - `cargo sqlx prepare --check` (`docs/hr-sandbox.md`)

### T2 - `write_run_memory` and usage counters (foundation)
- **Action**
  - D4: the verb in both stores and every forwarder.
  - D5: the tally in `finish_run` in both stores.
- **Tests first** (store conformance cases, registered in `CASES` and `run_case_inline`)
  - `run_memory_creates_a_run_entry_with_stamped_provenance`
  - `run_memory_supersedes_a_live_run_entry_on_the_folded_subject`
  - `run_memory_never_replaces_a_person_entry`
  - `run_memory_stops_at_the_write_cap`
  - `run_memory_refuses_a_step_that_is_not_capture`
  - `run_memory_refuses_a_lost_fence`
  - `finish_run_counts_memory_usage_once`: two steps show the entry, one done and one failed, giving
    `shown 2, done 1, failed 1`. A second `finish_run` is refused and leaves the counts as they are.
  - `finish_run_skips_deleted_or_unlisted_entries`
  - Pins: 290 + 8 in `mem_store.rs:36` and `pg_conformance.rs:63`, prose included.
- **Check**
  - `cargo test -p htui-core --features test-support -- --test-threads=1`
  - `cargo test -p htui-store --features test-support --test pg_conformance -- --test-threads=1`
  - `cargo test -p htui-agent --features test-support --no-run`
  - clippy and fmt for those three crates
  - `cargo sqlx prepare --check`

### T3 - Light-model setting and its Settings row (lane B)
- **Action**: everything in D7.
- **Tests first**
  - `settings.rs`:
    - `light_model_accepts_null_and_an_agent_model_object`
    - `light_model_refuses_extra_fields_a_bad_uuid_and_an_empty_model`
    - `resolve_light_model_is_none_when_unset_or_malformed`
    - the moved pins
  - `htui/tests/prompt_settings.rs`:
    - the row shows `not set`
    - the edit act cycles through enabled agents' pairs, then `not set`
    - an agent with no models and no default is left out
    - `not set` clears the row
    - the 80-column snapshot
- **Check**
  - `cargo test -p htui-core --features test-support --lib prompt::settings`
  - `cargo test -p htui --features testkit --test prompt_settings -- --test-threads=1`, reviewing the moved
    `insta` snapshots
  - clippy and fmt for `htui-core` and `htui`

### T4 - Capture core (pure)
- **Action**: the new module `htui-core/src/capture.rs`, covering D9 and D10.
- **Tests first** (`htui-core/tests/capture.rs`)
  - **Trigger**: fires on each of the four signals; does not fire on a clean run or on a judge-only failure.
  - **Pack**:
    - walk-order labels;
    - the win attempt is included;
    - stays under the cap, cutting tails first.
  - **Prompt**: the digest is stable.
  - **Parser**:
    - keeps a valid entry;
    - refuses whole a body with unknown fields, a body over the cap, and a body that is not JSON;
    - drops entries with a bad kind, no evidence, an unknown block, a quote under 12 characters, a quote not
      found in its block, or an unscrubbable subject;
    - drops the fourth entry as `OverCap`;
    - maps labels to `StepId`.
- **Check**
  - `cargo test -p htui-core --features test-support --test capture --lib`
  - clippy and fmt

### T5 - Run-written entries in prompts and the pane
- **Action**: D6 and D12.
- **Tests first**
  - `prompt_memory.rs`:
    - a run entry renders `source="run" trust="model-written"`;
    - mixed person and run entries keep newest-first order under one cap.
  - `settings_memory.rs`: the source column, as snapshots.
- **Check**
  - `cargo test -p htui-core --features test-support --test prompt_memory`
  - `cargo test -p htui --features testkit --test settings_memory -- --test-threads=1`
  - clippy and fmt

### T6 - Engine: freeze, `end_run`, the capture step
- **Action**
  - D8 and D11.
  - Add `capture: None,` at every `GraphSnapshot` literal.
  - Fake scripting under the phase name `capture` (attempt 1). An unscripted capture plays `"scripted output"`,
    which is not JSON, so it fails as a refused output. That is the intended result for unscripted runs.
  - Add a new `capture_case` group, chained in `case()`.
  - Box the capture future if the debug stack needs it.
  - Fix existing orch tests whose exact step lists move because a failing run now ends with a skipped capture
    row (D11 step 3).
- **Tests first** (orch conformance)
  - `a_clean_run_writes_no_capture_row`
  - `a_retried_run_captures_an_entry_with_provenance`: verify fails, the retry passes, and the document cites
    the failing block. The entry has `source = run`, plus `run_id`, `item_id`, agent, model and evidence.
  - `a_capture_without_a_light_model_is_skipped_on_its_row`
  - `capture_off_writes_no_row`, with the project switch off
  - `a_failed_capture_leaves_the_run_done`
  - `a_failed_run_still_captures_and_stays_failed`
  - `a_cancelled_run_never_captures`
  - `a_cancel_during_capture_cancels_the_capture_row`
  - `a_capture_citing_text_not_in_the_pack_writes_nothing`
  - `a_person_entry_on_the_subject_is_kept_and_noted`
  - `a_crash_during_capture_interrupts_it_and_finishes_the_run`
  - `an_older_snapshot_without_capture_never_captures`
  - Pins: 234 + 12 in `conformance.rs:17139` and in `tests/fake_conformance.rs:18` and :79, prose included.
  - `finish_run_is_the_last_write` still holds.
- **Check**
  - `cargo test -p htui-orch --features test-support -- --test-threads=1`
  - `cargo check --workspace --all-features --tests`, which compiles every literal site; this is not the gate
  - `cargo clippy -p htui-orch --features test-support -- -D warnings`
  - `cargo fmt --check`

## Validation
```bash
bash scripts/gate.sh                       # main thread only, after lanes A+B merge and after T6
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A failing run without a light model now ends with a skipped capture row, so step lists in existing suites (orch, `htui-worker`, `htui`) move | High | T6 fixes orch; the main-thread gate catches `htui-worker` and `htui`, and a gate repair fixes them. Alternative at CONFIRM: no row when no model is configured |
| A missed end path | Low | 16 sites listed by name (O1); queued and chat ends are argued above; the reviewer gets the list |
| Crash recovery on a failure path that does not reach `end_run` again | Medium | At-most-once guard; tests for the done path and `settle_failed`; the reviewer is told |
| Debug stack depth in engine walks (2 MiB) | Medium | Box the capture future; `--test-threads=1`; grep for SIGABRT |
| Prompt injection through command output | Medium | Quote must be in the pack; scrub; write cap; quoted-data rendering; `PersonHolds`. Residual risk is MOD-166's |
| `insta` snapshot churn | High | Review the moved snapshots |
| Migration renumbering at collect | High | Renumber at merge and move the D1a pins |

## Verified claims
| # | Claim | Verdict | Evidence |
|---|---|---|---|
| S1 | Next migration is 0031 | TRUE | `migrations/` ends at `0030_link_kinds.sql`; `schema_version()` is the max version (`pg/mod.rs:757`) |
| S2 | No cache migration needed | TRUE (amended) | Mirror has no CHECKs (`cache_migrations/0010:3`); memory not mirrored (`cache/read.rs:1316`); `refresh_run_step` column list is unchanged. Missed `node_key` comment pin added to D1 |
| S3 | `STAGE_KEYS` maps to positions | FALSE, amended | No mapping exists; `run_step_refusal` hard-codes `-3..=-1` (`traits.rs:2899`). Three -4 tests move (D3) |
| S4 | `finish_run` runs once, atomically | TRUE | `legal_move` in `mem.rs:8134` and `pg/write.rs:1797-1826` (`FOR UPDATE`); `close_batch` tallies nothing |
| S5 | trim_record memory JSON shape | TRUE | `trim.rs:128-154, 276`; stored raw (`run.rs:354`), Serialize-only, so the tally reads raw JSON |
| S6 | Fenced step write exists | TRUE (amended) | `pg/write.rs:9613`, `step_scope` :330, MemStore :8265, `StepFence` `traits.rs:4162`; the verb reads step and run rows too (D4) |
| S7 | Engine writes via `WorkerStore` | TRUE | Implementors MemStore, PgStore, Writer; `WriteStore` also UsageSpy, SpyStore; no Backend impl |
| S8 | `MemoryEntry` literal sites | PARTLY, amended | Three literals; the bigger cost is four `query_as!(MemoryEntry…)` sites, so D2 adds a flat row struct |
| S9 | Migration pins | PARTLY, amended | Nine count pins and the column-comment exactness test, now listed in D1/D1a |
| S10 | Pg uses offline sqlx macros | TRUE | `pg/read.rs:1119` `query_as!`; `.sqlx/` present |
| S11 | Feature names | TRUE | `test-support` (enables `demo`) on core, store, orch, agent; `testkit` on htui |
| O1 | All item-run ends go through orch `finish_run` | PARTLY, amended | 16 sites listed in D11; queued ends (`cancel_queued` :2657, `close_batch`, `claim_run`) and chat ends argued out |
| O2 | Adding snapshot fields is resume-safe | TRUE (amended) | No `deny_unknown_fields`; `topology_v2` hashes phases and edges only. About 13 `SnapshotSettings` literals, so the freeze moved to `GraphSnapshot.capture` beside `memory` (D8) |
| O3 | `resolve_or_refused` has app, project and agent lookup | TRUE | `graph.rs:495`, `GraphSource::agent` ~:113, literal :642-652 |
| O4 | `output_of` takes a kind | PARTLY, amended | It takes `&SnapshotPhase`; D11 builds a synthetic capture phase; `open_recorder` needs `AssembledPrompt` (D10) |
| O5 | A row can settle `cancelled` with a reason | FALSE, amended | No such verb; D11 uses `pass_step` (`done` + `skipped` + note) and `interrupt_step` for failures |
| O6 | Engine can create stage rows via `create_step` | TRUE | `NewRunStep` carries role, position and `node_key`; checked by `run_step_refusal` |
| O7 | Engine helpers callable from a sibling module | FALSE, amended | All private (`engine.rs:9160, 8967, 9709, 9614, 9514`); D11 uses child module `engine/capture.rs`; judge is untimed, so a new `CAPTURE_DEADLINE` |
| O8 | Fake can script a capture document | TRUE (conditional) | Only via `sink.after_done` (fake `write_output` :3040 writes `phase.output_kind`); D11 calls it |
| O9 | `light_model` sorts at 9; seed pins | PARTLY, amended | Index 9 true; no unseeded mechanism, filters by name; full pin list in D7 |
| O10 | Settings › Prompt can see agents | FALSE, amended | `SettingsSnapshot` is `{app, projects}`; D7 adds `agents` to the snapshot read; own pair builder filters enabled |
| O11 | T3 is disjoint from the other tasks | TRUE (amended) | No T3 file in another list; T3 also owns `prompt/mod.rs` re-export |
| O12 | Case counts 234 and 290 | TRUE | `conformance.rs:17139`, `fake_conformance.rs:18, 79`; `mem_store.rs:36`, `pg_conformance.rs:63` |

## Acceptance
- [ ] T1..T6 complete, each task's check green
- [ ] Gate green (sandbox Qdrant timeouts owed on host)
- [ ] rust-reviewer run over the full range; findings applied or deferred with the maintainer
- [ ] PRD milestones 1 and 2 complete
