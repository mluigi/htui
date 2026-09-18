# Plan: MOD-4 milestone 1 — the seam knows what a run is

**Source**: `.claude/prds/mod-4-orchestrator-manual-mode.prd.md`, milestone 1 only (as amended:
the PRD's milestones 1 and 2 are merged, see **D0**). Design authority: the PRD's D1–D8 (cited as
**PRD Dn**; this plan's own decisions are plain **Dn**), `docs/ANA-2.md` §4.1/§4.2/§4.3/§4.6/§4.7/
§4.9/§5.1/§5.4/§8/§9, `docs/ANA-5.md` §4.6.
**Requirements**: `R-ENT-8`, `R-ENT-10`, `R-ENT-12`, `R-ORCH-1`, `R-ORCH-8`, `R-ORCH-9`,
`R-ORCH-11`, `R-TUI-9`; `R-NF-3` by construction (no UI in this milestone).
**Complexity**: Large (18 `WriteStore` methods on six impls, 5 `ReadStore` reads on eight, 11 new
inherent reads, one migration pair, 14 new conformance cases).
**Routing**: routed as **PRD** by `/handoff-run MOD-4` (criteria C2, C3, C4 fired). Ultracode
recommended for implement and review; **the maintainer scoped it to implement only**. Reviewer:
`rust-reviewer` (`.claude/workflow-config.json:2`).
**Status**: **complete, 2026-09-18** (`33277b1`..`e3163ca`, 37 commits). Workspace green at
`--test-threads=1`: 1304 passed, 0 failed, 63 binaries; `cargo sqlx prepare --check` exit 0;
clippy and fmt clean. Implemented as three ultracode workflows (T1/T2/T3), each closing with an
adversarial verify fan-out whose confirmed findings were fixed before the next task began; the
`rust-reviewer` gate then blocked on one HIGH (`select_fanout` forcing a failed judge to `done` and
erasing its `gate_note`, against ANA-2 §4.5), which is fixed in `d622814`. Two of its findings are
recorded for milestone 2 in the blueprint's §4.6 (R-1, R-2) rather than implemented here: both
argue against settled decisions of this plan.

## Summary

Teach the store seam what a graph run is. `WriteStore`
(`crates/htui-core/src/store/traits.rs:139`) grows from 42 methods to 60: run creation and the
admission transaction, the lease and its sweep, step creation and the two status compare-and-sets,
step settlement, gate answers, fan-out selection, supersession, isolation trees and commit hashes,
document writes with the version allocated inside the transaction, promotion, run failure,
close-out and the refusal note. `ReadStore` grows by five reads the SQLite mirror can answer, one of
which is ANA-2 §4.2's input resolver that `documents_of_kinds` deliberately is not
(`traits.rs:97`). Beneath them, `Status`, `RunStatus` and `StepStatus` gain `can_move_to` as a
`const fn`, and `transition` stops accepting any pair whose `from` happens to match. `PgStore`
implements all of it over `0003_orchestration.sql` and its `cache_migrations/0003` companion,
`Backend` gains the eleven inherent reads of ANA-2 §8 that do not already exist, and the refresher
gains its eleventh cursor-driven table. No engine, no git, no agent; the only `htui` code touched is
one test helper and one string of confirmation copy.

## D0 — Why milestones 1 and 2 are one milestone

The PRD draws milestone 1 as "no Postgres". That cannot close green. `WriteStore` and `ReadStore`
carry **no default bodies** — `BufferedWriter`'s impl (`writer.rs:233`) is exhaustive by
construction and MOD-25 kept it compiling on purpose — so the first new trait method stops
`htui-store` compiling, and `PgStore` cannot implement one until `0003` exists, because
`SQLX_OFFLINE = "true"` (`.cargo/config.toml:5-6`) prepares `query!` against real columns. MOD-15
met the same wall and absorbed it inside a single milestone, recording that `cargo test --workspace`
is red between its tasks (`mod-15-hierarchy-seam.plan.md:109-118`). Defaulted bodies were rejected
there and are rejected here: a silently no-op arm is how `MemStore` and `PgStore` come to disagree.
The seven-milestone shape of PRD D1 therefore becomes six; ANA-2 §9's build order is untouched, one
cut point is removed, and this milestone spans its steps 1 and 2.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | **The eighteen writers go on `WriteStore`; five reads go on `ReadStore`; eleven reads are `Backend`-inherent with a `match self`.** ANA-2 §8's inherent block names sixteen, and the fact-check found **five already exist**: `agents()` (`backend.rs:302`), `app_settings()` (`:394`) and `item_kind(id)` (`:413`) on `Backend`; `phases(graph)` (`traits.rs:537`) and `repos(project)` (`:429`) on `WriteStore`, where MOD-15 put them so its conformance suite could read back what it wrote. **ANA-2 §8 and the shipped seam therefore disagree about where `phases` and `repos` belong, and the shipped placement wins** — moving a method MOD-15's cases call to satisfy a table would break working code to match a document. `prompt_template(project, name, version)` is a near-miss: `prompt_templates(project)` exists at `backend.rs:346` with a different signature, so the single-row form is one of the eleven. The split test is ANA-2 §8's and is already law in this tree: a mirrored table's read is a trait method, an unmirrored table's read is inherent (`traits.rs:21-23`). `run`, `run_step`, `run_step_commit` are mirrored and `run_step_tree` becomes mirrored in this milestone, so `run(id)`, `run_steps(run)`, `step_trees(step)`, `step_commits(step)` are trait reads. `step_graph`, `step_graph_phase`, `phase_agent`, `prompt_template`, `agent`, `agent_box`, `box`, `repo`, `repo_box_path`, `app_setting` are not mirrored, so their sixteen reads are inherent, following `agents()` (`backend.rs:302-308`). | The conformance suite is generic over the trait and cannot call an inherent read; a read that is not on the trait cannot be asserted on both stores. The inverse is equally binding: `CacheStore: ReadStore` must be able to answer every trait read, and it cannot answer a table it does not mirror. |
| D2 | **ANA-2 §8's resolver is a new method, not a widened `documents_of_kinds`.** `resolve_inputs(item, run, kinds) -> Vec<Document>` implements §4.2's statement in full — `s.selected IS NOT FALSE` to exclude fan-out losers, `ORDER BY (s.run_id = $run) DESC NULLS LAST, d.version DESC`, one row per kind, a missing kind reported as such. The shipped `documents_of_kinds` (`traits.rs:98`) keeps its signature and its callers. | `traits.rs:97` says it in as many words: the shipped method "is **not** ANA-2 §8's resolver: it prefers no run's output and excludes no loser, because both need `run_step` rows MOD-4 owns. MOD-4 layers that on top." Widening it would change MOD-2's prompt path, which has no run to prefer. |
| D3 | **`can_move_to` is a `const fn` per enum in `htui-core`, and `transition` gains one guard.** `Status::can_move_to`, `RunStatus::can_move_to`, `StepStatus::can_move_to` encode §4.3's three tables as exhaustive `match` expressions. `WriteStore::transition` keeps its signature, its `Ok(false)` on a stale `from`, its `NotFound` on a missing row and its never-bumps-`version` behaviour, and gains: an illegal `(from, to)` returns `StoreError::Constraint` **without performing the update**. `transition_run` and `transition_step` are the same shape for the other two enums. | ANA-2 §4.3 verdict (`docs/ANA-2.md:667-687`) specifies exactly this, including that the third case extends the shipped `status_cas_keeps_version` rather than duplicating it. A legality check in the engine instead of the seam would let a direct store caller — the demo loader, a test, MOD-13's editor — write a state the engine cannot reason about. |
| D4 | **The shipped suite pins three illegal pairs, and they are not the ones this plan first named** (corrected by the fact-check, F1). The real breakage set is **six call sites in three files**: `conformance.rs:558` (`Open → Done`, asserted `Ok(false)` — the case exists to prove a **stale `from`**, so its replacement must stay stale while becoming legal, e.g. `Queued → InProgress` against a row already moved on); `conformance.rs:580` (`Queued → Done`, asserted to succeed — `queued` reaches only `in_progress`, `open` or `blocked`); `conformance.rs:649` (`Open → Closed`, asserted to succeed, in `no_delete_path`); `pg_criteria.rs:487` (`Open → Done`, the Postgres twin of the version assertion); `pg_criteria.rs:535` (`Open → Done` on an unknown id, asserted `NotFound` — see **D14**); `writer_buffered.rs:251` (`Open → InProgress`, asserted `Unreachable` — see **D15**). **Three sites this plan previously listed are already legal and need no edit**: `conformance.rs:537` (`Open → Queued`), `:596` (`Done → Open`, the reopen row), `:872` (`Blocked → Open`), as are `pg_criteria.rs:504` and `:528`. | A case that asserts an illegal transition succeeds fails the moment the law lands. The `:558` case is the subtle one and the reason this needed checking rather than asserting: it does not test legality at all, it tests that a stale `from` returns `Ok(false)`, and a careless "move it to a legal pair" edit would delete the property it exists to prove. |
| D14 | **`NotFound` beats `Constraint`: the row is looked up first, legality second.** An illegal `(from, to)` against a row that does not exist returns `StoreError::NotFound`, not `Constraint`. | ANA-2 §4.3 fixes the refusal but is silent on precedence (`docs/ANA-2.md:667-687`), and `pg_criteria.rs:535` already pins `NotFound` for exactly that combination. Choosing legality-first would break a shipped assertion to satisfy an unwritten rule, and would also tell a caller its pair is wrong when the real problem is that its id is. |
| D15 | **The guard lives in a shared helper in `traits.rs` called by `MemStore` and `PgStore`, not at the trait's entry.** `BufferedWriter` keeps refusing unconditionally before any legality question is asked, so `writer_buffered.rs:251`'s `Open → InProgress` assertion stays green and stays honest: offline, the pair never gets that far. | The precedent is `chat_step_status` (`traits.rs:617-625`) — one rule, one home, both stores calling it, so they cannot disagree about what is legal. A guard at the seam entry would make an offline call return `Constraint` instead of `Unreachable`, which is the wrong sentence for the wrong reason. |
| D16 | **Two shipped divergences from §4.3 are recorded here and deliberately not fixed.** MOD-2's chat path inserts `run` at `'running'` and `run_step` at `'running'` (`pg/write.rs:721`, `:736`) where §4.3's insert rows say `queued` and `pending`, and the offline upload path inserts both straight at `'done'` (`cache/pending.rs:518`, `:554`); `finish_chat_run` is an unconditional `UPDATE` with no `from` predicate (`pg/write.rs:775`, `:792`), so a replayed close performs `done → done`, which no table sanctions. | §4.3's three tables describe a **graph** run's lifecycle; a `kind='chat'` run has no graph, no phase and no gate, and it is created and closed in one shape MOD-2 settled. Widening the law to cover it, or narrowing MOD-2 to fit the law, is a change to shipped behaviour that this milestone has no requirement for. Recorded so milestone 2's engine does not assume every `run` row in the database passed through `can_move_to`. |
| D5 | **`BufferedWriter` refuses all eighteen writers with `StoreError::Unreachable(DATABASE_UNREACHABLE)`; `Writer` delegates.** No new constant, no real buffered implementation. The five new `ReadStore` reads refuse there too. | MOD-15's D2 precedent, and `writer.rs:640-652` already records that MOD-25's one sentence serves reads as well as writes. `htui` is online-only and CLEAN-2 deletes `BufferedWriter` outright; a buffered `claim_run` would be code written to be deleted. `writer.rs:325-333` reserved this exact moment: "when MOD-4 first does, the refusal is the signal that an offline graph step needs a design". |
| D6 | **Five methods are transactions, and they are transactions on `MemStore` too.** `create_run` (insert `run` + move `item.status` + write `graph_snapshot`), `claim_run` (the §4.7 admission: `SELECT … FOR UPDATE` on the box row, the `running` count, the overlap check, the status and lease update), `select_fanout` (winner, every loser, the judge's note), `close_out` (summary document at the next version, `item → closed`, refusal while any run is non-terminal) and `write_document` (version allocated inside the insert under the row lock). `MemStore` runs each inside one `write` closure so a partial outcome is impossible there either. | ANA-2 §4.7's admission is the critical section the whole overlap rule rests on; §4.5's bookkeeping is "one transaction" verbatim; `R-TUI-9` close-out is "one transaction, refusing while any run of the item is non-terminal". `MemStore`'s house rule already forbids `.await` under the lock (`mem.rs:3-7`), so the closure shape is the existing one. |
| D7 | **`0003_orchestration.sql` is ANA-2 §9's text, and the cache companion is `0003`, not `0002`.** Three `step_graph_phase` columns (`judge_agent_id`, `judge_model`, `deadline_seconds` + `ck_phase_judge_model`), four `run` columns (`repo_scope`, `lease_box_id`, `lease_owner`, `lease_expires_at`) with `ck_run_graph_snapshot` **`NOT VALID`** and two indexes, three `run_step` columns (`verify_outcome`, `verify_exit_code`, `promoted_at`), `step_graph.is_override` with its partial index, the `run_step_tree` table, ~13 `COMMENT ON COLUMN` statements and twelve idempotent `app_setting` rows. **No `CHECK (fanout_index >= 0)`, ever.** The mirror side adds the same columns to `run`/`run_step` (minus `lease_owner`, which is a liveness token for a process that is by definition not running), the `run_step_tree` mirror table, **and — beyond ANA-2 §9's text — `probed_tags`, `declared_tags` and `settings` on the mirrored `box` table**, because `BoxInfo` gains those three fields (D10) and `cache/read.rs:845-849` builds a `BoxInfo` from a `SqliteRow` that would otherwise have nothing to read (fact-check F14). | `cache_migrations/0002_agent_mirror.sql` is MOD-2 milestone 4's, which ANA-2 §9 predates; `0002_agent_probe.sql:6-8` states the correction in its own header. `NOT VALID` is required because the fixture writes graph runs with a NULL snapshot today. The `fanout_index` prohibition is ANA-2 risk 12 and `HANDOFF.md:199`: `-1` is the judge step. |
| D8 | **Five shipped assertions and four copies of one string are amended in the migration's own commit** (corrected by the fact-check, F9/F10/F11). Migration pins: `migrations.rs:18` (`const TABLES`, gains `run_step_tree`) and `:89-93` (`TABLES.len() == 32`) — **neither of which this plan first named**, and `:95-99`'s "32 tables and nothing else" is derived from `TABLES.len()` so it moves for free; `migrations.rs:70-74` (`applied == vec![1, 2]`, not `:69-73`); `:258-260` ("exactly the six commented columns, and no others"); `:353-357` and `connect.rs:93-97` (`MigrationState::Pending(2)`). The mirrored-table count is a **string in four places**, not one: `connection.rs:136` (the constant), `connection.rs:989` (its in-file unit test), `tests/connection.rs:1664`, and `tests/snapshots/connection__confirm.snap:32` — **so this milestone re-records one UI snapshot**. `cache/mod.rs:31`'s "sixteen mirrored tables" doc goes stale with them. **`cache.rs:1318`/`:1330` do _not_ need amending**: that test opens at literal `1` and asserts `2` (`cache.rs:69`), a self-contained pair that still passes after `0003`. | Each is a true statement about a two-migration, sixteen-table tree. Amending them together makes one commit that says "the schema moved"; amending them lazily makes five unrelated-looking red tests in three crates. The `cache.rs` correction matters in the other direction: this plan first claimed it would break, and editing a passing test to match a wrong claim is how a suite loses meaning. |
| D9 | **The mirror's eleventh table lands at all three call sites, not the one ANA-2 cites.** `run_step_tree` joins `MIRRORED_TABLES` (16 → 17, `cache/mod.rs:41-58`), the cursor-driven const list (`refresh.rs:200-210`), **the array literal at `refresh.rs:254-265` and the match at `:271-282`** — the two ANA-2 §9 does not name. It rides its parent step's `updated_at` exactly as `run_step_commit` does, so it needs no trigger and gets none. | ANA-2 §9 cites `refresh.rs:200-210` alone; the file has grown two more sites since. A table in the const list and absent from the match is a mirror that compiles and never refreshes. |
| D10 | **Projection additions are appended, never inserted.** `RunStepSummary` gains `usage`, `selected`, `exit_code`, `verify_outcome`, `promoted_at` and a denormalised `agent_name` (ANA-2 §6.2); `ItemSummary` gains `touched_paths`, which `Item` already carries (`item.rs:63-64`) so `Item::summary()` fills it directly; `BoxInfo` gains `probed_tags`, `declared_tags` and `settings` (§4.7). Every new `query_as!` field goes at the **end** of the select list. **Construction sites, counted by the fact-check**: `RunStepSummary` has the three that must agree — `pg/rows.rs:125-141`, `mem.rs:828-845`, `cache/read.rs:537` — plus a fourth in a unit test (`detail/runs.rs:549-558`) that uses functional-update syntax and is therefore unaffected. `BoxInfo` has **three**, none using `..`: `pg/read.rs:701-714` (positional `query_as!`), `mem.rs:225-229`, `cache/read.rs:845-849` (needs D7's new mirror columns). `ItemSummary` has **four**: `item.rs:84-96`, `pg/read.rs:81-95`, `cache/read.rs:169-182`, and **`crates/htui/src/app/update.rs:416-427`**, a test helper naming all eleven fields with no `..` — which is why T1 is not quite `htui-core`-only. | `pg/rows.rs:114-120` warns in the file that the binding is positional, and a conformance case asserts the three `RunStepSummary` builders agree field for field. `BoxProfile` is deliberately *not* widened: it drops tags on purpose (`box_.rs:97-101`). |
| D11 | **`BoxSettings` and `ProjectSettings` are read-only types this milestone.** Both live in `htui-core` with `#[serde(default)]` on every field, deserialised from the existing `box.settings` / `project.settings` blobs. No writer is added: MOD-15 already ships a key-level `set_setting` over `SettingRung` and a whole-blob round trip would erase keys (MOD-15 PRD D7). Unknown keys survive because nothing re-serialises the struct. | ANA-2 §4.7/§5.2 define the two structs and `0003` adds only `COMMENT ON COLUMN` for them. The one thing this milestone must not do is give `project.settings` a second writer with different merge semantics. |
| D12 | **Fourteen conformance cases, grouped by concern; `CASES` 36 → 47, `READ_CASES` 6 → 9.** Three writer cases were added by the fact-check's findings: one pinning **D14**'s precedence (`NotFound` before `Constraint`), one pinning that `MemStore` holds `run_step_commit` rows it has never held before, and one pinning the three `RunStepSummary` builders against each other. `READ_CASES` is pinned in exactly **one** place (`mem_store.rs:42-46`) — `pg_conformance.rs` has no `EXPECTED_READ_CASES` and iterates the slice without a count assertion. Writers: `run_create_moves_the_item`, `claim_run_admits_one_and_refuses_the_second`, `lease_refresh_is_a_cas_on_owner`, `step_create_and_transition_law`, `finish_step_records_the_settle`, `gate_answers_write_their_outcome`, `select_fanout_is_one_transaction`, `trees_and_commits_round_trip`, `write_document_allocates_its_version`, `close_out_refuses_a_live_run`, `illegal_transitions_are_constraint`. Reads: `run_and_steps_round_trip`, `trees_and_commits_read_back`, `resolve_inputs_prefers_this_run_and_skips_losers`. Column-level facts the mirror cannot answer stay in `pg_criteria.rs`, named from the case doc comment so `every_cross_referenced_test_name_exists` (`conformance.rs:4066-4132`) resolves them — and that function's filename table (`:4104-4112`) knows only `conformance`, `mem` and `pg_criteria`, so no case doc may name a fourth file. | The Postgres runner creates and drops one database per case (`pg_conformance.rs:35-42`): today 42 cycles, after this milestone 53. A case per method would be 60 cycles for the writers alone. MOD-15 plan D12 set the precedent of grouping by concern, and `EXPECTED_CASES` (`pg_conformance.rs:19`) plus the two `mem_store.rs` pins move once. |
| D13 | **The two outstanding fixture corrections land here; the two MOD-15 already made are struck from the item.** `attempt: 0 → 1` on five step rows (`fixtures.rs:1296`, `:1330`) and a real `graph_snapshot` on both graph runs (`:1234`, `:1251`) — ANA-2 cites `:1113`/`:1130`, which the file outgrew. `review` in `implement.input_kinds` and the `gate_hard` seeding are **already applied** (`seed.rs:87-91`, `:65-86`), so `HANDOFF.md:187-188`'s list of four loses two at close-out. The two `attempt` literals produce five rows (`:1330` is inside `fn done_step`, called four times at `:1258-1268`), and the fact-check confirmed **the corrections break nothing**: no `.snap` renders `attempt` or `graph_snapshot`, no test asserts either value, the demo round-trip is value-blind, and the mirror parity tests compare both sides so they move together. The plan's first draft claimed the snapshot would be "built by the same `graph.rs` code path the engine will use" — **false**: no `GraphSnapshot` type exists yet and the only `graph.rs` in the tree is a TUI pane `htui-core` cannot depend on. The fixture's snapshot is built from the type T1 defines in `model/run.rs`, which is also what milestone 2's builder will construct. | `ck_run_graph_snapshot` is `NOT VALID` precisely because of these rows, and `attempt: 0` contradicts `0001_init.sql:476`'s own default. `the_fixture_phases_are_the_seed_rows` (`fixtures.rs:1692-1729`) means the seed and the fixture are one edit, not two. |

## Patterns to Mirror

| Concern | Mirror | Where |
|---|---|---|
| Naming | `New*` request structs beside their row; `*Outcome` result enums; case names as full sentences | `model/item.rs` (`NewItem`), `model/run.rs:201-224` (`ChatRunSpec`), `traits.rs:691-745` (`UpdateOutcome`, `CasOutcome`, `DeleteReach`) |
| Status enums | `str_enum!` with `is_terminal`/`is_active` helpers beside the variants | `model/run.rs:31-77`, `model/item.rs:8-36` |
| Errors | `StoreError::{NotFound{entity,id}, Constraint(String), Unreachable(&'static str)}`; `map_sqlx` turns `23xxx` into `Constraint`; seam-side rules refuse through a named helper fn | `store/error.rs`; `htui-store/src/error.rs:17-52`; `traits.rs:617-681` (`chat_step_status`, `graph_not_in_project`, `item_kind_is_held`) |
| Refusals | One constant, reused by name; never a second sentence for the same fact | `writer.rs:604`, `:622-623`, `:638-655` |
| Data access (Pg) | single-statement `query!`/`query_as!` with `AS "col: Type"`, `.map_err(map_sqlx)`, `rows_affected() == 0` → `NotFound`, `pool.begin()` only for multi-row writes, `RETURNING` reads the trigger's `updated_at`, **never `SET updated_at`** | `pg/write.rs:826-845` (`set_step_prompt`), `:495-522` (`append_events`, `jsonb_to_recordset`) |
| Data access (Mem) | `self.read(\|state\| …)` / `self.write(\|state\| …)`, no `.await` under the guard, every refusable rule in `impl State` | `mem.rs:3-7`, `:2856-2858`, `:1264` (`start_chat_run`) |
| Data access (JSONB) | read the whole document, never a typed round trip that could drop a key | `mem.rs:296-299`, `pg/read.rs` `project_settings` |
| Inherent read | three-arm `match self`, one like-named method per concrete store, `Offline` refusing through a named helper | `backend.rs:302-308` (`agents`), `:394-400` (`app_settings`), `:423-425` (`prompt_offline`) |
| Mirror read | `CacheStore` answers the trait; a table absent from `MIRRORED_TABLES` is answered by nobody | `cache/read.rs:225`, `cache/mod.rs:41-58` |
| Migration | forward-only, header naming the ANA and the item, `COMMENT ON COLUMN` for every semantic the DDL cannot carry, `ON CONFLICT DO NOTHING` seeds | `migrations/0002_agent_probe.sql:1-79` |
| Tests | `async fn case<S: WriteStore>(store: &S)` with the case name in every assertion message; `CASES` + a `run_case` arm + the count pins; per-backend twins named in the doc comment | `conformance.rs:79-135`, `:4036-4154`; `mem_store.rs:35-46`; `pg_conformance.rs:19-28`; `pg_criteria.rs` |
| Test naming | full-sentence snake_case describing the asserted behaviour | `crates/htui-store/tests/*`, e.g. `a_broken_keyring_starts_offline_instead_of_refusing_to_launch` |

## Files to Change

| File | Action | Task | Why |
|---|---|---|---|
| `crates/htui-core/src/model/item.rs` | edit | T1 | `Status::can_move_to`; `ItemSummary.touched_paths` |
| `crates/htui-core/src/model/run.rs` | edit | T1 | `RunStatus::can_move_to`, `StepStatus::can_move_to`; `NewRun`, `NewRunStep`, `StepOutcome`, `RunStepTree`, `GraphSnapshot` shape; `RunStepSummary` +6 fields |
| `crates/htui-core/src/model/box_.rs` | edit | T1 | `BoxInfo` +`probed_tags`, `declared_tags`, `settings`; `BoxSettings` |
| `crates/htui-core/src/model/kind.rs` | edit | T1 | `ProjectSettings`; `is_override` on the graph row type |
| `crates/htui-core/src/model/mod.rs` | edit | T1 | re-export the new types |
| `crates/htui-core/src/store/traits.rs` | edit | T1 | 18 `WriteStore` methods, 5 `ReadStore` reads, the `Constraint` guard on `transition`, new helper refusals |
| `crates/htui-core/src/store/mem.rs` | edit | T1 | `State` gains **`step_trees` and `step_commits`** — `MemStore` holds none of the three run tables today (`traits.rs:741-743`); 23 impls plus the five transactional closures |
| `crates/htui-core/src/store/conformance.rs` | edit | T1 | 14 new cases, `CASES` 36 → 47, `READ_CASES` 6 → 9, `run_case`/`run_read_case` arms; the three illegal-pair call sites of D4 |
| `crates/htui-core/tests/mem_store.rs` | edit | T1 | both count pins (36 → 47, 6 → 9) |
| `crates/htui/src/app/update.rs` | edit | T1 | the `items_reply` test helper names all `ItemSummary` fields with no `..` (D10) |
| `crates/htui-core/src/fixtures.rs` | edit | T1 | `attempt` 1-based ×5; `graph_snapshot` on both graph runs (D13) |
| `crates/htui-store/migrations/0003_orchestration.sql` | create | T2 | ANA-2 §9's DDL |
| `crates/htui-store/cache_migrations/0003_orchestration.sql` | create | T2 | the mirror side (D7) |
| `crates/htui-store/src/pg/write.rs` | edit | T2 | 18 writers, five of them transactions |
| `crates/htui-store/src/pg/read.rs` | edit | T2 | 5 trait reads + 16 inherent reads |
| `crates/htui-store/src/pg/rows.rs` | edit | T2 | projection rows, fields **appended** (D10) |
| `crates/htui-store/src/backend.rs` | edit | T2 | 16 inherent `match self` arms + the 5 trait reads |
| `crates/htui-store/src/cache/mod.rs` | edit | T2 | `MIRRORED_TABLES` 16 → 17 |
| `crates/htui-store/src/cache/refresh.rs` | edit | T2 | the const, the array literal and the match (D9) |
| `crates/htui-store/src/cache/read.rs` | edit | T2 | the 5 trait reads off the mirror |
| `crates/htui-store/.sqlx/query-*.json` | add | T2 | offline data for every new query |
| `crates/htui-store/tests/migrations.rs` | edit | T2 | `TABLES` + its length pin + the three pins of D8 |
| `crates/htui-store/tests/connect.rs` | edit | T2 | `Pending(2)` → `Pending(3)` (`:93-97`) |
| `crates/htui-store/tests/cache.rs` | edit | T2 | the mirror's new read cases and the `run_step_tree` refresh test. **Not** the two `schema_version` literals — they pass unchanged (D8) |
| `crates/htui-store/tests/pg_conformance.rs` | edit | T2 | `EXPECTED_CASES` 36 → 47 |
| `crates/htui/src/ui/tabs/settings/connection.rs` | edit | T2 | the constant at `:136` **and** its unit test at `:989` (D8) |
| `crates/htui/tests/connection.rs` | edit | T2 | the `"16 mirrored tables"` expectation at `:1664` |
| `crates/htui/tests/snapshots/connection__confirm.snap` | re-record | T2 | the rebuild copy is rendered into it (`:32`) |
| `crates/htui-store/tests/pg_criteria.rs` | edit | T2 | Postgres-only twins: `FOR UPDATE` admission under concurrency, `NOT VALID` constraint behaviour, version allocation under contention |
| `crates/htui-store/src/writer.rs` | edit | T3 | `Writer` delegation; `BufferedWriter` refusals (D5) |
| `crates/htui-store/tests/writer_buffered.rs` | edit | T3 | every new method answers MOD-25's one sentence |
| `crates/htui-agent/src/conformance.rs`, `crates/htui-agent/tests/recorder.rs` | edit | T3 | `UsageSpy` and `SpyStore` arms so the driver suite still compiles |
Not touched, on purpose: `crates/htui-orch` (does not exist until milestone 2), `run_worker.rs`
(milestone 6), every `ui/tabs/backlog/**` file, `agent_worker.rs`, `seed.rs` (its two ANA-2
amendments already landed), and MOD-2's chat write path (D16).

## Tasks

**T1 → T2 → T3, fully serial**, and the reason is the same one MOD-15 recorded: the traits have no
default bodies, so `htui-store` does not compile between T1's first commit and T2's last. Each
task's crate-scoped gate is the live signal; `cargo test --workspace` is the acceptance gate and is
red in between. **Three serial tasks are not a workflow, so ultracode is not used for this
milestone** despite the routing recommendation — there is nothing to fan out. The recommendation
stands for milestones 3 to 6, where the isolation modes, the fan-out paths and the Runs-tab actions
do decompose. TDD per task: the test that fails for the stated reason comes first.

Every implementer prompt carries: PRD D1–D8 win over this plan where they disagree; graphify-first
for codebase questions, with every graph-derived fact re-verified against the tree; `.sqlx`
regenerated and committed with any query change; nothing sets `updated_at` by hand; no new refusal
constant; no second migration; commit incrementally, because uncommitted work does not survive the
session.

### Task 1: `htui-core` — types, the status law, `MemStore`, conformance
- **Action**: write the fourteen cases and their `CASES`/`READ_CASES` entries first, plus the two
  `mem_store.rs` pins; they fail to compile. Then the three `can_move_to` tables, the request and
  outcome types, the new `Run`/`RunStep` columns (`repo_scope`, the three lease fields,
  `verify_outcome`, `verify_exit_code`, `promoted_at`), the projection additions (appended),
  `BoxSettings`/`ProjectSettings`, the 18 + 5 trait signatures with their doc comments,
  `MemStore`'s impls with the five transactional closures and its two new `State` maps, and the
  shared legality helper of **D15** with **D14**'s precedence. Amend **D4**'s three `conformance.rs`
  call sites in the same commit as the law — `:558` keeps testing a stale `from`, which is the one
  edit that can silently lose a property. Then `crates/htui/src/app/update.rs:416-427`, the one
  `htui` file this task touches. Finish with the fixture corrections of **D13**.
- **Mirror**: `mem.rs`'s closure discipline; `conformance.rs`'s case shape; `str_enum!` helpers;
  `chat_step_status` as the shape of the legality helper.
- **Validate**: `cargo test -p htui-core --all-features`, `cargo clippy -p htui-core --all-targets --all-features -- -D warnings`.

### Task 2: `htui-store` — the migration pair, `PgStore`, `Backend`, the mirror (after T1)
- **Action**: write `0003_orchestration.sql` and the cache companion (including D7's three mirrored
  `box` columns); amend **D8**'s five assertions and all four copies of the mirrored-table string,
  re-recording `connection__confirm.snap`, in the same commit. Then `PgStore`'s 18 writers (five as
  transactions), the 5 trait reads, the **11** new inherent reads, `Backend`'s `match self` arms,
  `CacheStore`'s five reads, the mirror's three refresh sites (**D9**), and the projection rows with
  fields appended (**D10**).
  Run `cargo sqlx prepare` from inside `crates/htui-store` and commit `.sqlx/` with the query that
  needed it, never after.
- **Mirror**: `pg/write.rs`'s single-statement style and `map_sqlx`; `backend.rs:302-308`.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test -p htui-store --all-features`,
  then `cargo sqlx prepare --check -- --all-targets --all-features` from `crates/htui-store`.

### Task 3: `Writer`, `BufferedWriter` and the two spies (after T2)
- **Action**: delegation arms on `Writer`; `Unreachable(DATABASE_UNREACHABLE)` on all 23 new
  methods of `BufferedWriter`; arms on `UsageSpy` and `SpyStore` so `htui-agent`'s suite compiles;
  assertions that every new method answers MOD-25's one sentence and not a second one.
- **Mirror**: `writer.rs:640-655`; `writer_buffered.rs:308-326`.
- **Validate**: `cargo test --workspace --all-features` (the acceptance gate, now expected green).

## Validation

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features -- --test-threads=1
cargo doc --workspace --no-deps
cd crates/htui-store && \
  DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| `htui-store` is red for the whole of T1 and most of T2, and a real regression hides in the noise | Certain | Crate-scoped gates per task; T1 ends with `-p htui-core` green, T2 with `-p htui-store` green, T3 with the workspace green |
| `run_step_tree` lands in the const list and not in the refresh match, so the mirror never populates it | Medium | D9 names all three sites; a cache test asserts a tree row reaches the mirror after a refresh |
| An appended `query_as!` field is inserted mid-list and every projection silently shifts | Medium | `pg/rows.rs:114-120`'s own warning quoted in the task; a conformance case asserts the three `RunStepSummary` builders agree field for field |
| The `COMMENT ON COLUMN` pin ("exactly six, and no others") is amended by deleting the assertion rather than extending the list | Low | D8 names it as the largest blast radius; the amended test lists all ~19 comments by name |
| `claim_run`'s `FOR UPDATE` admission is correct on Postgres and meaningless on `MemStore`, so the conformance case proves nothing about concurrency | High | The concurrency half lives in `pg_criteria.rs` as a two-connection test; the conformance case asserts the *decision* (second run stays `queued`), which both stores can make |
| A 60-method `WriteStore` becomes unreviewable in one pass | Medium | Three commits minimum per task, grouped by concern in the same order as the conformance cases |
| `schema_version` bumping to 3 rebuilds every developer mirror on first run and reads as data loss | Certain | Documented behaviour; the close-out note says it plainly |

## Acceptance

- [x] All three tasks complete, each committed incrementally
- [x] `CASES` 47 / `READ_CASES` 9, both pins moved, `run_case` exhaustive
- [x] Both migrations applied on a clean database; `cargo sqlx prepare --check` green
- [x] Every new offline **refusal** answers MOD-25's single sentence — `BufferedWriter`'s
      twenty-three and `Backend::Offline`'s eleven inherent orchestration reads, pinned by
      `writer_buffered.rs`'s `every_run_seam_method_is_unreachable_offline` and
      `every_inherent_orchestration_read_is_unreachable_offline`. Not "every new offline *path*":
      the five new `ReadStore` reads (`run`, `run_steps`, `step_trees`, `step_commits`,
      `resolve_inputs`) all read a `MIRRORED_TABLES` table, so plan D12's read-dispatch rule has
      `Backend::Offline` answer them **off the mirror** rather than refuse; they refuse only on
      `BufferedWriter`, which no backend has handed out since MOD-25
- [x] Validation block passes at `--test-threads=1` with Postgres live, **except
      `cargo doc --workspace --no-deps`**, which exits 101 on six pre-existing `htui-store`
      intra-doc links (`dsn.rs` ×4, `secret.rs` ×2). MOD-4 added none of them: `git diff
      0cf232d..HEAD` touches neither file, and `cargo doc -p htui` is exit 0. Not this milestone's
      to fix — a CLEAN item; see blueprint §4.6
- [x] Patterns mirrored, not reinvented

## Verified claims (fact-check, 2026-09-18)

Every verifiable claim this plan makes, checked against the tree at HEAD `0cf232d` before the
CONFIRM gate. Six were falsified and the plan above is the amended version; the originals are
stated so the correction is auditable.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| F1 | `Open → Done` is asserted to succeed at `conformance.rs:537`, `:558`, `:580`, `:596`, `:649`, `:872` | **FALSIFIED** | Only `:558` is `Open → Done` and it asserts `Ok(false)`, not success. Actual pairs: `:537` Open→Queued (legal), `:558` Open→Done (stale-`from` case), `:580` Queued→Done (illegal, asserted true), `:596` Done→Open (legal, reopen), `:649` Open→Closed (illegal, asserted true), `:872` Blocked→Open (legal). D4 rewritten |
| F2 | The breakage set is confined to `conformance.rs` | **FALSIFIED** | `pg_criteria.rs:487` (`Open → Done`, asserted true) and `:535` (`NotFound` on an unknown id) and `writer_buffered.rs:251` (`Open → InProgress`) are also affected. D4, D14, D15 added |
| F3 | ANA-2 fixes the precedence of `Constraint` against `NotFound` | **FALSIFIED** | `docs/ANA-2.md:667-687` specifies the refusal and is silent on precedence; `pg_criteria.rs:535` already pins `NotFound`. D14 decides it |
| F4 | `finish_chat_run` only performs moves §4.3 permits | **CONFIRMED, with divergences elsewhere** | `chat_step_status` (`traits.rs:617-625`) permits only `Done\|Failed\|Cancelled` and both backends mint at `running` (`pg/write.rs:721`, `:736`), so every move is `running → terminal`. But the **inserts** land at `running`/`running` where §4.3 says `queued`/`pending`, the upload path inserts at `done` (`cache/pending.rs:518`, `:554`), and `finish_chat_run`'s `UPDATE` has no `from` predicate (`pg/write.rs:775`, `:792`). D16 records all three |
| F5 | `WriteStore` is 42 methods; `ReadStore` is 11 | **CONFIRMED** | `traits.rs:139-609` (42 `async fn`, `mint_item` `:141` … `delete_project` `:608`); `traits.rs:63-127` (11) |
| F6 | `document(id)`/`documents_of_kinds` already exist, so only four §8 reads are new plus the resolver | **CONFIRMED** | `traits.rs:85`, `:98`; `run`, `run_steps`, `step_trees`, `step_commits`, `resolve_inputs` have zero hits workspace-wide. `mem.rs:817`'s `run_steps` is a private `State` helper, not the seam |
| F7 | Six `WriteStore` impls, eight `ReadStore` impls | **CONFIRMED** | Writes: `writer.rs:233`, `:752`, `pg/write.rs:233`, `htui-agent/src/conformance.rs:645`, `mem.rs:2780`, `htui-agent/tests/recorder.rs:323`. Reads: those minus none, plus `backend.rs:430`, `pg/read.rs:58`, `cache/read.rs:225` |
| F8 | `CASES` 36 and `READ_CASES` 6, each pinned twice | **PARTIAL** | Counts confirmed (`conformance.rs:31-68`, `:169-176`). `CASES` is pinned at `pg_conformance.rs:19` and `mem_store.rs:37`; **`READ_CASES` is pinned once**, at `mem_store.rs:44` — `pg_conformance.rs` has no `EXPECTED_READ_CASES`. D12 amended |
| F9 | `Backend` gains sixteen inherent reads | **FALSIFIED** | Five of ANA-2 §8's sixteen exist: `agents()` `backend.rs:302`, `app_settings()` `:394`, `item_kind(id)` `:413`, and `phases(graph)`/`repos(project)` on `WriteStore` (`traits.rs:537`, `:429`). At most **eleven** are new, and §8 disagrees with the shipped placement of two. D1 and the complexity line amended |
| F10 | ANA-2 §8's write table is 17 rows / 18 names | **CONFIRMED** | `docs/ANA-2.md:1739-1755`; `:1744` names `transition_run` **and** `transition_step`. Its own prose at `:1760` ("sixteen") contradicts the table |
| F11 | The inherent block is 14 rows / 16 names | **CONFIRMED** | `docs/ANA-2.md:1711-1724`; `:1717` and `:1719` each name two |
| F12 | `MemStore`'s `State` needs a `step_trees` map | **PARTIAL** | True but incomplete: `State` (`mem.rs:53-119`) has **no `run_step_commit` collection either**, and `step_commits(step)` is going on `ReadStore`. `traits.rs:741-743` says `MemStore` "holds none of the three tables". Files-to-change amended |
| F13 | `connection.rs:136` hard-codes "the 16 mirrored tables" | **CONFIRMED, blast radius understated** | The same string lives at `connection.rs:989` (unit test), `tests/connection.rs:1664`, and `tests/snapshots/connection__confirm.snap:32` — **so this milestone re-records a UI snapshot**, which the PRD said only milestone 6 would. `cache/mod.rs:31`'s doc goes stale too. D8 amended, PRD amended |
| F14 | `cache.rs:1318`/`:1330` pin `schema_version == 2` and must be amended | **FALSIFIED** | Both literals exist, but the test opens the store at literal `1` (`cache.rs:69`) and asserts `2` — a self-contained `1 → 2` pair that still passes after `0003`. Removed from the required edits |
| F15 | The four `migrations.rs` pins are the whole migration blast radius | **PARTIAL** | `:70-74` (not `:69-73`), `:258-260` and `:353-357` confirmed; **`const TABLES` at `:18` and its length pin at `:89-93` were missed**, and `:95-99` is derived from `TABLES.len()` so it follows for free |
| F16 | The fixture corrections break something that must be sequenced | **FALSIFIED — they break nothing** | No `.snap` renders `attempt` (`detail/runs.rs:282-290` renders `position`) or `graph_snapshot` (zero hits in every `.snap`); no test asserts either value; `demo.rs:401-438` binds both straight through; `load_demo_round_trips_a_count_per_table` counts rows only; mirror parity tests compare both sides. Also: two literals produce five rows, not five literals |
| F17 | The fixture's snapshot is built by "the same `graph.rs` code path the engine will use" | **FALSIFIED** | No `GraphSnapshot` type exists (zero hits) and the only `graph.rs` is a TUI pane `htui-core` cannot depend on. D13 corrected |
| F18 | `RunStepSummary` is built in exactly three places | **PARTIAL** | The three are correct; a fourth literal at `detail/runs.rs:549-558` uses functional-update syntax and is unaffected by appended fields |
| F19 | `BoxInfo` is three fields with three construction sites | **CONFIRMED, with a consequence** | `box_.rs:85-92`; sites `pg/read.rs:701-714` (positional `query_as!`), `mem.rs:225-229`, `cache/read.rs:845-849` — **the mirror has no `probed_tags`/`declared_tags`/`settings` columns**, so `cache_migrations/0003` must add them. D7 amended |
| F20 | `ItemSummary` gains `touched_paths` from `Item` | **CONFIRMED** | `Item.touched_paths` exists (`item.rs:63-64`), `ItemSummary` lacks it (`:102-127`). Four construction sites, one of them `crates/htui/src/app/update.rs:416-427` with no `..` — so T1 touches one `htui` file |
| F21 | `every_cross_referenced_test_name_exists` knows three filenames and panics on a fourth | **CONFIRMED** | `conformance.rs:4066-4132`, table at `:4104-4112` |
| F22 | `0003_orchestration.sql` and `cache_migrations/0003_orchestration.sql` do not exist; the cache companion is `0003` not ANA-2's `0002` | **CONFIRMED** | Only `0001`/`0002` in both directories; `0002_agent_probe.sql:6-8` states the correction |
| F23 | Task independence | **N/A — nothing is marked independent** | T1 → T2 → T3 are declared serial for the compile reason in D0; their file sets are stated per row in Files to Change and are not relied on for parallelism |
