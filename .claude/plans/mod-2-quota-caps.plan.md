# Plan: MOD-2 live `agy` turn, quota and caps (milestone 6 close-out + milestone 7)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestones**: 6's outstanding task (`T34`) and 7 (Quota and caps). Milestones 1–2 landed
under `.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`), milestone 4 under
`.claude/plans/mod-2-durable-history-replay.plan.md` (`81d247b`), milestone 5 under
`.claude/plans/mod-2-probe-autodiscovery.plan.md` (`fb626a8`), milestone 6 under
`.claude/plans/mod-2-agy-acp.plan.md` (`acf16f7`, one task short). This plan continues their
decision (`D65`+) and task (`T37`+) numbering, and **carries `T34` under its original id** — it is
the same task, not a new one. Milestones 8 and 9 are out of scope; where one of them owns a seam
this plan touches, the plan says where it stops.

**Design authority**: `docs/ANA-4.md` §7 (what each transport can report, the `usage` payload
reconciliation, the `agent_box.quota` blob shape, the passive latch, `R-AGT-8`'s skip rule, the
per-token cap enforcement point), §9 (`agent_box.quota` "was already reserved for exactly the §7
blob"; **MOD-12 enforces the batch cap**, ANA-4:1283), §4.5 and §11.14 (the three open `agy`
items `T34` closes), §11 criteria 7, 8 and 11. `docs/ANA-2.md` §7 (MOD-4's skip predicate reads the
probe). `docs/REQUIREMENTS.md` `R-AGT-7`, `R-AGT-8`, `R-AGT-5`, `R-TUI-8`, `R-NF-3`, `R-SEC-3`.

**Requirements**: `R-AGT-7` (remaining allowance per agent, refreshed per run; a configurable
per-run cap enforced by cancelling the session), `R-AGT-8` (the quota state a candidate-selection
loop reads), `R-AGT-5` (nothing added here may be keyed on an agent name — the quota *source* is
declared in the row), `R-TUI-8` (the registry's Settings section shows it), `R-NF-3` (no blocking
work on the UI task), `R-SEC-3` (a quota blob is a payload and is scrubbed like one).

**Complexity**: Medium

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-10 with the maintainer accepting the
verdict (only C3 fired — the three empirical `agy` unknowns, answered by running the real adapter
rather than by a document) and choosing **one plan covering `T34` + milestone 7**. Ultracode: not
needed; the chain is short and its two halves are gated on one live binary.

**CONFIRMED 2026-09-10.** The maintainer accepted the plan as written, including every decision
`D65`–`D73` and, explicitly, **D69's ANA-4 §7 amendment** (detection stays in the recorder, the
cancel moves to `pump`) — an ANA change is maintainer-only, and this is the record of it, kept here
rather than applied to `docs/ANA-4.md`, per the milestone-5 and -6 precedent. Also confirmed: D70's
per-project cap read with no env stand-in, D71's deferral of the batch cap to MOD-12, D73's
untouched Chat tab, and D65's ordering. Model staffing at CONFIRM: `code-architect` on
**Fable 5.1**, implementers on **Opus 5**, the `rust-reviewer` gate on **Fable 5.1**. Work runs on
`feat/mod-2-quota-caps`, cut from `main`.

## Summary

Two things are true at once, and the second depends on the first.

**`T34` is unblocked.** MOD-21 logged `agy_acp_server` in from inside `htui` on 2026-09-10
(`docs/decisions/mod/mod-21.md`); `~/.gemini/antigravity-acp/acp_token.json` exists on this box, so
`session/new` now succeeds and `agy_live.rs` passes 4/4. What milestone 6 still owes is **a live
turn**, which none of the four existing cases drives, and with it three ANA-4 §11.14 answers:
whether `agy_acp_server` emits `usage_update` and in what field, whether it issues
`session/request_permission` in `default` mode and with what option ids and kinds, and whether its
edits arrive as a standard `tool_call` + `diff` or in a vendor shape.

**Milestone 7 is the accounting those answers feed.** Today the tree computes usage but reports no
allowance and enforces no cap:

- `agent_box.quota` and `quota_at` have existed since `0001_init.sql:119-120` and **nothing ever
  writes them**. The one code path that could — `WriteStore::upsert_agent_box`
  (`traits.rs:117`) — writes the whole row, `probe` column included.
- The ACP mapper never reads `_meta` at all (`acp/map.rs:74-107` builds `UsageEvent` from `used`,
  `size` and `cost` only), so the rate-limit blob milestone 3 *observed* live under
  `_meta["_claude/rateLimit"]` is dropped on the floor. `QuotaSource::AcpMetaRateLimit`
  (`launch.rs:388-389`) is a declared setting with no reader.
- `per_token_cap_run` and `per_token_cap_batch` are documented `project.settings` JSON keys
  (`0001_init.sql:150-151`) with **no reader and no enforcement anywhere**: `cap_exceeded` appears
  once in the tree, as a doc-comment example on `ErrorEvent.code` (`event.rs:367`).
- The Settings agents table renders seven columns and none of them is quota
  (`ui/tabs/settings/agents.rs:973-980`).

So milestone 7 is: capture the blob the wire already carries, latch it without clobbering the
probe's work, enforce the run cap where every `usage` row is seen, show the result, and hand MOD-4
the predicate `R-AGT-8` describes.

One structural fact reshapes ANA-4 §7's stated mechanism and is the plan's load-bearing decision
(D69): §7 says "the recorder … on breach, calls `AgentSession::cancel()`", but `Recorder`
(`record.rs:211-243`) holds no session and no reference to one — `pump` holds both the session and
the recorder (`record.rs:965-982`). Detection stays where §7 puts it; the cancel moves one layer out
to the only place that can perform it.

## Prerequisites (state of this box, verified 2026-09-10)

| Fact | Value |
|---|---|
| `agy_acp_server` credential | `~/.gemini/antigravity-acp/acp_token.json` **present** (MOD-21's live login) |
| Adapter | `~/.local/share/htui/agents/antigravity-acp/1.1.1/` (milestone 6 T29) |
| Dev Postgres | `htui-postgres` up, `0.0.0.0:5439->5432` |
| Test DSN | `HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres`, `USERNAME=htui-ci` (TOOL-2) |
| Baseline suite | `cargo test --workspace --all-features` exits 0 on this tree (`main`, 2026-09-10) |
| Branch | cut `feat/mod-2-quota-caps` from `main`; `main` is clean apart from untracked `.serena/` |

`T34` spends real model tokens against the maintainer's own subscription credential (D63). It stays
`#[ignore]` by default like every live test in the tree.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D65 | **`T34` runs first and its answers are inputs to the rest of the plan, not a footnote after it.** Whether `agy_acp_server` emits `usage_update` at all decides whether its seed keeps `quota.source: "none"` and whether a second transport exercises the latch. If it emits nothing, that is the recorded answer and `agy`'s quota column reads `—` by design. | ANA-4 §7 marks the `agy` row of its own table "**Unverified - MOD-2 must confirm**", and §11.14 lists it. Writing the latch first and then discovering the shape would be the failure mode ANA-4 §12's live-probe discipline exists to prevent — milestone 6 already paid for guessing once (`--uid=`). |
| D66 | **The vendor rate-limit blob is captured verbatim onto `UsageEvent`, and normalized outside the mapper.** `UsageEvent` gains `quota: Option<Value>` (`#[serde(skip_serializing_if)]`), filled from `_meta["_claude/rateLimit"]` when present. No new `DriverEvent` variant. Normalization into §7's blob lives in a new `htui_core::model::quota`, selected by the row's declared `agent.settings.quota.source` — never by agent name. | D62's precedent, unchanged: the eleven variants are closed, and a vendor shape is data, not a variant. `UsageEvent` is where a per-turn observation belongs; adding a key is explicitly permitted (ANA-9 §4.3, and ANA-4 §7 already adds four). `R-AGT-5` forbids the alternative: a `match agent.name` here is the "second hard-coded agent" the PRD's risk table names, and `QuotaSource` (`launch.rs:376-397`) is the declarative slot that already exists for it. Keeping the raw blob means a vendor field this milestone does not understand is still recorded on the row rather than discarded. |
| D67 | **The latch writes through a new narrow `WriteStore::set_agent_box_quota(agent_id, box_id, quota, quota_at)`, not `upsert_agent_box`.** | `upsert_agent_box` writes `probe` too (`pg/write.rs:418-436`), so latching quota through it would either clobber the probe snapshot or force the writer to carry a full row it never read. It would also race the re-probe paths D55/D60 introduced: a chat can re-probe its own row mid-session, and two writers of one row must not be one statement wide. The narrow setter is two columns and cannot lose a snapshot. |
| D68 | **The latch is best-effort and never fails a chat; offline it is a documented no-op.** A failed quota write is logged and dropped, not propagated to the turn. `Writer::Buffered` has nowhere to put it: the cache mirror has no `agent_box` table at all, deliberately (`cache_migrations/0002_agent_mirror.sql:9-11`), and `agent_worker.rs:1521-1531` already refuses registry writes offline with `REGISTRY_ON_SERVER_ONLY`. An offline chat therefore leaves the last server-side quota value standing, and the pending buffer carries the `usage` rows that will re-derive it after upload. | `R-AGT-7` is a reporting requirement; `R-HIS-1` (nothing about a session exists only in memory) is a durability requirement, and it is satisfied by the `usage` rows, which *are* buffered. Failing a turn because an advisory allowance figure could not be stored would trade the requirement for the courtesy. |
| D69 | **Cap detection stays in the recorder; the cancellation is performed by the loop that holds the session.** *(Amended after the blueprint — see P-1: `pump` is the conformance suite's loop, `run_turn` is the binary's, so the cancel-and-close sequence is one shared `record::enforce_breach(session, recorder, breach)` called from both.)* This is an ANA-4 §7 amendment, recorded here rather than in the ANA** (the milestone-5 and -6 precedent for maintainer-only files). `Recorder` gains the cap inputs and, after each `usage` row, returns a breach verdict; `pump` — which holds `&mut dyn AgentSession` and the recorder together (`record.rs:965-982`) — calls `cancel(grace)`, then records `error { code: "cap_exceeded", message }` followed by `done { stop_reason: Cancelled }`. | §7's claim that the recorder "calls `AgentSession::cancel()`" is not implementable as written: `Recorder` (`record.rs:211-243`) has no session field, no session import, and giving it one would put the transport back inside the recorder that milestone 1 deliberately kept transport-neutral (it is the type the conformance suite drives with no transport at all). The *reason* §7 names the recorder — it is the only place that sees every `usage` row — is about detection, and detection is exactly what stays. Criterion 8's two observable claims ("within one event of the breach", "the last two rows") are unaffected by which layer makes the call. |
| D70 | **Caps are read from `project.settings` through a new inherent `project_settings(ProjectId)` on each store, dispatched by `Backend` — the `agents()` shape (`traits.rs:11-14`, `backend.rs:292`) — and no migration is added.** `per_token_cap_run` is USD **micros**, matching `cost_micros`; an absent key means no cap, not a cap of zero. The `KEEP_RAW_ENV` stand-in precedent (`agent_worker.rs:78-82`) is deliberately **not** copied. | The keys are already documented `project.settings` JSON (`0001_init.sql:150-151`), `agent_box.quota` already exists (ANA-4:1277), and MOD-4 has reserved `0003_orchestration.sql` — so a migration here would either collide with that reservation or be a fourth file in a forward-only chain for data that needs no DDL. Micros because a float cap compared against an integer running total is a rounding argument waiting to happen, and because `cost_micros` is what the recorder already sums (`usage.rs:24-38`). Env is right for `keep_raw_events` (a developer's debugging switch) and wrong for a cap: `R-AGT-7` says *configurable*, and the row it is configured on is the project. |
| D71 | **The per-**run** cap is enforced here; the per-**batch** cap is not built.** `project.settings.per_token_cap_batch` is read and reported as unenforced-here, and the run-level accounting the batch cap needs is what this milestone exposes. | ANA-4 §9 assigns it: "MOD-12 enforces the batch cap using §7's accounting" (ANA-4:1283). A batch spans runs MOD-4 does not yet create; enforcing it here would mean inventing the batch identity, which is `R-ORCH` territory. |
| D72 | **`R-AGT-8` ships as a pure predicate, not a selection loop.** `htui_core::model::quota::available(quota: Option<&Value>, spend: Option<i64>, cap: Option<i64>) -> Availability` implements §7's four skip rules (`exhausted`, any window `utilization >= 1.0`, `status != "allowed"`, cap already reached) with **null quota meaning available**. MOD-4 calls it; MOD-2 tests it. | §7's selection rule is written for "the orchestrator", which is MOD-4 (`docs/ANA-2.md` §7 owns the skip predicate and already reads `probe->>'status'` beside it). A loop here would be a second orchestrator; a predicate here is the thing MOD-4 cannot write without the blob's shape, and `agy`-reports-nothing is precisely why null must be available (§7 says so in as many words). |
| D73 | **Settings gains an eighth `quota` column and says out loud that `r` cannot refresh it. The Chat tab is not touched.** The column renders the tightest window (`utilization` as a percentage plus its reset) for a subscription row, session spend for a per-token row, and `—` when the row reports nothing. | `R-TUI-8` puts the registry in Settings and ANA-4 §7 requires the refresh limit be "stated in the UI rather than implied" — a probe handshake reports no quota, so `r` genuinely cannot. The Chat tab stays out because a per-turn cost line is not in `R-AGT-7` and a cap breach is already visible there: it arrives as the `error` row the chat renders today, with `done{cancelled}` behind it. Adding a cost widget would be scope this milestone did not buy. |

| D74 | **`agent_box.quota` and `quota_at` become single-writer: only `set_agent_box_quota` can write them, and `upsert_agent_box` loses the ability entirely.** The two columns leave both the `INSERT` list and the `ON CONFLICT … DO UPDATE SET` list (`pg/write.rs:418-436`), so a fresh row is inserted with them NULL and an upsert of an existing row cannot touch them; `MemStore` preserves the stored values instead of `*stored = row.clone()` (`mem.rs:839-848`); and `probe::agent_box_row` stops carrying them forward from `existing` (`probe.rs:1527-1528`), since there is no longer anything to carry them *to*. Added at the maintainer's instruction on 2026-09-10, replacing the blueprint's "accepted and self-healing" treatment of H-6. **MOD-7 inherits this**: box registration writes quota through the narrow setter, not through the row (the note in `agent_box_row`'s doc changes with it). | The probe's own doc already says "the probe owns neither" (`probe.rs:1504-1505`) and then hands both to a statement whose `SET` list writes them — a read-modify-write over a column another writer owns, which is a lost update by construction, not a race that needs unlucky timing: the probe reads `existing` at chat start and writes it back seconds later, discarding every latch in between. Self-healing on the next `usage` row was true but not good enough — a turn's *last* report is the one a chat leaves behind, and that is exactly the value the Settings column shows until the next chat. Making the column single-writer is cheaper than making two writers agree, and it is enforced by the schema access pattern rather than by a comment asking callers to be careful. |

| D75 | **ANA-4 §11 criterion 6's dedup rule is fixed here, not deferred: the `edit_proposal` index becomes step-scoped instead of flush-scoped.** `Recorder.edits` (`record.rs:226`) is cleared by `flush()` (`record.rs:646`), so "one `edit_proposal` row per `(tool_call_id, path)` per step" (ANA-4 §4.3) holds only until the first flush. T34 proved it broken on a real transport: one `agy` file write produced **three** rows (`seq` 12, 15, 16), two sharing the key, because `agy` re-announces `tool_call` verbatim where the spec's example sends `tool_call_update`, and the permission park/answer flushes in between. Folded into milestone 7 at the maintainer's instruction (2026-09-10) as **T46**, because T39/T40 already have `record.rs` open. | Criterion 6 is one of the thirteen MOD-2 must pass, and it currently passes **only against the fake**, which never flushes mid-call — the transport-neutral suite could not see this, which is itself worth recording: a conformance case is only as good as the interleavings its script produces. MOD-2 cannot honestly report criterion 6 at close-out with a live counter-example in its own fixture directory. The index has to survive a flush because a flush is a persistence detail and `(tool_call_id, path)` identity is a domain fact; making the dedup key outlive the buffer is the smaller change of the two available, and the row's `seq` is already stable once written. |
| D76 | **The `default` column shows the whole `default_model` string; the packing is rebalanced to pay for it.** Maintainer's instruction, 2026-09-10: a truncated model id is not acceptable, because the id *is* the coordinate ANA-4 §4.4 selects by — `gemini-3.` names nothing. `default` therefore fits `gemini-3.7-flash-high` (21 chars) at a 100-column render, and the width is reclaimed from the other columns in this stated priority order: **the full default model wins; then the quota window's reset time; then `on this box`'s slack.** T47. | T41 kept `default` at `Length(9)` and spent the new room on quota (`Length(19)`), which was the right call *before* `agy`'s seed carried a 21-character id — D64 changed the facts under it. The eight columns cannot all be full at 98 usable columns (they want ~107), so this is a real trade and the maintainer has now ranked it rather than leaving it to whoever renders last. Recording the priority order matters more than the numbers: MOD-23's editor and MOD-12's caps section will both want room in this same table. |

| D77 | **T46 is fixed by reserving the `seq` at announcement and deferring only the *write* (maintainer's choice of option (d), 2026-09-10).** An `edit_proposal` takes its `seq` the moment the agent announces it, exactly as today, but the row is held instead of flushed; a re-announcement of the same `(tool_call_id, path)` updates the held row in place; the row is written when the tool call closes. **No store update seam** is added and `upload_pending`'s idempotency is untouched. | The three rejected options each cost more than this one. (a) a new `update_event_payload` across six `WriteStore` impls *plus* choosing between "criterion 6 online only" and making the offline upload last-line-wins — which is the rule protecting `UsageTotals::from_rows` from double-counting. (b) deferring the whole row including its `seq` reorders the transcript. (c) suppressing byte-identical repeats fixes the observed `agy` case and none of the general one, which is how criterion 6 came to be believed in the first place. What makes (d) work is that **persistence and display are already separate**: the recorder sends its UI frame at announcement (`send_ui`) and only the database write waits, so nothing the user sees moves, and replay reads by `seq`, which was never given up. The price is one crash window — a process death mid-tool-call loses a proposal row that is durable today — and the maintainer accepted it knowingly. |
| D78 | **M-2: `nothing_to_say` reads billing as well as source.** A row whose `billing` is `per_token` publishes its spend on the first costed usage row, whatever its declared `quota.source`, because ANA-4 §7 gives a per-token agent no `windows` — so there is nothing for the H-3 rule to protect and nothing to wait for. A `subscription` row keeps the source-based rule verbatim. | The realistic instance is a `claude` row switched to API-key billing while `settings.quota.source` still reads `acp_meta_rate_limit`: the allowance blob never arrives, so the H-3 guard waits forever and the column reads `—` while the recorder is holding the exact number §7 asks it to publish. Found by the review gate (M-2), not by a test — the combination is one no fixture had. |

## Patterns to Mirror

- `crates/htui-core/src/model/usage.rs` — a pure model module: nullable integer fields, saturating
  adds, a hand-built `to_value()`, and a test module that pins the JSON shape. `model::quota` is its
  sibling and its `to_value()` is the §7 blob.
- `crates/htui-store/src/backend.rs:292` + `mem.rs:223` + `pg/read.rs:538` + `cache/read.rs:730` —
  `agents()`: one inherent method per store, three `Backend` arms, no trait. D70's
  `project_settings` follows it exactly, including the offline arm (the mirror already carries
  `project.settings`, `cache_migrations/0001_mirror.sql:57-61`).
- `crates/htui-agent/src/conformance.rs:1092` (`usage_deltas_sum_to_step_usage`) and `:810`
  (`cancel_answers_parked_permissions`) — the two existing cases the cap case sits between:
  transport-neutral, driven through `UsageSpy` (`conformance.rs:434-528`) and a real `cancel()`
  call (`conformance.rs:835`).
- `crates/htui/tests/chat_live.rs` — `T34`'s model: `#[tokio::test(flavor = "multi_thread")]`,
  `#[ignore]` with a reason, the production `AgentRuntime`, a `pgrep` survivor check, and store-row
  assertions through `step_events`.
- `crates/htui-agent/tests/fixtures/claude_acp_turn.jsonl` + `tests/replay.rs:392-417` — how a live
  transcript becomes a replayable fixture; `T34`'s `agy` transcript is recorded the same way.

## Files to Change

| File | Change |
|---|---|
| `crates/htui/tests/chat_live_agy.rs` | **new** — T34: one live `agy` turn through the production runtime, §11.14 answers asserted where they are assertions and printed where they are observations |
| `crates/htui-agent/tests/fixtures/agy_acp_turn.jsonl` | **new** — T34: the recorded turn |
| `crates/htui-core/seeds/agent_agy.json` | T34: `models`/`default_model`/`model_config_id` per D64 if `session/new` supplies them; `settings.quota.source` confirmed or corrected by what the turn shows |
| `crates/htui-agent/src/event.rs` | T37: `UsageEvent.quota: Option<Value>` (D66) |
| `crates/htui-agent/src/acp/map.rs` | T37: read `_meta["_claude/rateLimit"]` off `usage_update` into that field; mapper stays otherwise unchanged |
| `crates/htui-core/src/model/quota.rs` | **new** — T39/T42: the §7 blob type, `normalize(source, raw, billing, spend)`, `to_value()`, and D72's `available()` |
| `crates/htui-core/src/model/mod.rs` | T39: module wiring and re-exports |
| `crates/htui-core/src/store/traits.rs` | T38: `WriteStore::set_agent_box_quota` (D67) |
| `crates/htui-core/src/store/mem.rs` | T38: the setter; D70 inherent `project_settings` |
| `crates/htui-core/src/store/conformance.rs` | T38: **one** case, 20 → 21; T45: **one** more, 21 → 22 (this is the *store* suite; `pg_conformance.rs:19` pins the count) |
| `crates/htui-agent/src/probe.rs` | T45 (D74): `agent_box_row` stops carrying `quota`/`quota_at` forward from `existing`; its doc says why there is nothing to carry them to |
| `crates/htui-agent/src/conformance.rs` | T39/T40: **two** cases, 13 → 15 (this is the *agent* suite; `acp_conformance.rs:85` and `fake_conformance.rs:30` pin the count) |
| `crates/htui-store/src/pg/write.rs` | T38: two-column `UPDATE`, keyed `(agent_id, box_id)` |
| `crates/htui-store/src/pg/read.rs` | D70: `project_settings` |
| `crates/htui-store/src/cache/read.rs` | D70: `project_settings` off the mirror |
| `crates/htui-store/src/backend.rs` | D70: the three-arm dispatch |
| `crates/htui-store/src/writer.rs` | T38: `Writer` arm for the setter; `Buffered` refuses with `REGISTRY_ON_SERVER_ONLY` (D68) |
| `crates/htui-agent/src/record.rs` | T39: latch the normalized blob when a `usage` row carries one; T40: cap inputs, running-spend comparison, breach verdict out of `record()`; `pump` performs the cancel and writes the two closing rows (D69) |
| `crates/htui/src/agent_worker.rs` | T40: read the project's caps at `ChatStart` (D70) and hand them to the recorder; the cancel uses the existing `CANCEL_GRACE` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | T41: the `quota` column and the refresh statement (D73) |
| `crates/htui-agent/tests/{acp_map.rs,recorder.rs}` | T37/T39/T40 unit coverage; fixture-driven `_meta` capture |
| `crates/htui-store/tests/pg_criteria.rs` | T38: the setter against live Postgres, `probe` byte-identical across the write |
| `crates/htui/Cargo.toml` | T43: **one manifest change, against the blueprint's "no manifest changes" line** — `sqlx = { workspace = true }` as a `htui` **dev**-dependency. No store method returns `run_step.usage` (`pg_criteria.rs:93-105` says so) and `htui-store` re-exports no `sqlx`, so criterion 7's raw `SELECT` is otherwise unreachable from the one crate that can drive a `Mapper` **and** a `Recorder`. Runtime-checked queries only, so `.sqlx/` stays a `htui-store` concern and `cargo sqlx prepare --check` is unaffected; `Cargo.lock` gained one line. The alternative — a `testkit` helper in `htui-store` — was rejected to keep one task out of another's crate |
| `crates/htui/tests/chat_usage_pg.rs` | **new** — T43/T39: criterion 7's ACP clause and the latch against live Postgres. **Not** `pg_criteria.rs`: `htui-store` has no `htui-agent` dependency and cannot drive a mapper or a recorder, so `htui` — the one crate holding both — owns this proof (the `chat_offline.rs:1-14` precedent) |
| `crates/htui/tests/settings_agents.rs` (or the existing settings test module) | T41: the column and the statement, snapshot-pinned |
| `README.md`, `HANDOFF.md`, PRD milestone rows 6 and 7 | T44: close-out |

## Tasks

TDD per repo convention: the test that fails for the stated reason comes first. `T34` is the
exception in shape only — its "test" *is* the live probe, and it asserts what it can and records
what it observes (the `agy_live.rs:490-554` precedent).

### T34 (carried from milestone 6): a live `agy` chat through the production runtime — first, alone

`crates/htui/tests/chat_live_agy.rs`, `#[ignore]`, modelled on `chat_live.rs`. One turn against the
now-authenticated server, streamed through `AgentRuntime::production()`, asserting the same
structural floor `chat_live.rs` asserts (a `Prompt` row at `seq 0`, some `AssistantText`, a `Done`,
the session banner as the first `Other` row, no surviving `agy_acp_server`). Its research half
answers three §11.14 items, each recorded in this plan's answer table and in the eventual
`docs/decisions/mod/mod-2.md`:

1. **`usage_update`** — emitted at all? under which field? with `cost`, or context occupancy only?
2. **`session/request_permission` in `default` mode** — issued? with what option ids and
   `PermissionOptionKind` values? (The turn must ask for something permission-worthy to find out —
   a small file edit in a scratch directory.)
3. **The edit shape** — a standard `tool_call` with `kind: "edit"` and a `diff` content block, or a
   vendor shape landing in `other`?

Also re-runs `agy_live.rs` case 3 for the model list (D64) now that `session/new` succeeds. The
transcript is saved as a fixture. **The mapper is amended only if the wire demands it** (D62); a
demanded twelfth `DriverEvent` variant is a finding to surface, not a change to make quietly.

Files: `crates/htui/tests/chat_live_agy.rs`, `crates/htui-agent/tests/fixtures/agy_acp_turn.jsonl`,
`crates/htui-core/seeds/agent_agy.json`.

### T37: `_meta` rate-limit capture (independent; `event.rs`, `acp/map.rs`)

Test first, fixture-driven: a `usage_update` carrying `_meta["_claude/rateLimit"]` maps to a
`UsageEvent` whose `quota` holds that object verbatim, and one without it maps to `quota: None`.
Then D66's two-field change. `UsageTotals::add_payload` iterates five fixed keys
(`usage.rs:48-54`), so the new key cannot enter a sum — assert that too, because criterion 7 depends
on it.

Files: `crates/htui-agent/src/event.rs`, `crates/htui-agent/src/acp/map.rs`,
`crates/htui-agent/tests/acp_map.rs`, `crates/htui-agent/tests/fixtures/`.

### T38: the narrow quota setter (independent of T37; store crates only)

Test first, through the store conformance suite and against live Postgres: `set_agent_box_quota`
writes `quota`/`quota_at` and **leaves `probe` byte-identical** (the D67 claim, asserted rather
than assumed); a `Buffered` writer refuses with `REGISTRY_ON_SERVER_ONLY`. Then the method, its
three implementations and the `Writer` arm.

Files: `crates/htui-core/src/store/{traits.rs,mem.rs,conformance.rs}`,
`crates/htui-store/src/{pg/write.rs,writer.rs}`, `crates/htui-store/tests/pg_criteria.rs`.

### T39: the §7 blob and the passive latch (serial after T34, T37, T38 — `record.rs`)

Test first: a scripted session whose `usage` rows carry a rate-limit blob leaves `agent_box.quota`
equal to §7's document (`source`, `billing`, `status`, `exhausted`, `windows[]`, `spend`,
`observed_at`) with `quota_at == observed_at`; a row declaring `quota.source: "none"` latches
**spend only**; a store error on the latch does not fail the turn (D68). Then `model::quota`'s
`normalize`/`to_value` and the recorder's latch call.

`billing` comes from the `agent` row, `source` from `agent.settings.quota.source`
(`launch.rs:379-398`), `spend.session_micros` from the cumulative `cost_micros_total` the mapper
already tracks (`acp/map.rs:92-96`). Nothing reads an agent name.

Files: `crates/htui-core/src/model/{quota.rs,mod.rs}`, `crates/htui-agent/src/record.rs`,
`crates/htui-agent/tests/recorder.rs`.

### T40: run-cap enforcement (serial after T39 — same `record.rs`)

Test first, as a new transport-neutral conformance case (criterion 8): with a cap of *n* micros, a
session whose second `usage` row crosses *n* is cancelled **within one event**, and the step's last
two rows are `error{code:"cap_exceeded"}` then `done{stop_reason:"cancelled"}`, with the step marked
failed. A session under its cap is untouched; a project with no cap key is unbounded. Then D69's
verdict-out-of-`record()` plus the `pump` cancel, and the worker wiring that reads the caps at
`ChatStart` (D70) — off the UI task, on the chat's own task (`R-NF-3`).

Files: `crates/htui-agent/src/record.rs`, `crates/htui-core/src/store/conformance.rs`,
`crates/htui/src/agent_worker.rs`, plus D70's read path
(`crates/htui-core/src/store/mem.rs`, `crates/htui-store/src/{pg/read.rs,cache/read.rs,backend.rs}`).

### T45: single-writer quota columns (serial immediately after T38 — same files; D74)

Added at the maintainer's instruction, 2026-09-10. Test first, three of them:

1. store conformance `upsert_agent_box_cannot_write_quota` — latch a value through
   `set_agent_box_quota`, then `upsert_agent_box` the same row carrying a *different* `quota` and a
   `None` `quota_at`; the stored values are the latch's, unchanged. Insert-path half: a first
   `upsert_agent_box` carrying `quota: Some(..)` stores `NULL`, because the probe has no business
   seeding an allowance it never observed.
2. `pg_criteria.rs` — the same two claims against live Postgres, read back with a raw `SELECT`
   (the `EXCLUDED.quota` line is the thing being removed, so the proof has to be SQL-level).
3. `crates/htui-agent/tests/probe.rs` — `agent_box_row` returns `quota: None, quota_at: None` even
   when `existing` carries both, and every other field it projects is unchanged.

Then D74: the two columns out of both SQL lists (`pg/write.rs:418-436`), `MemStore` preserving the
stored pair rather than replacing the row wholesale (`mem.rs:839-848`), `agent_box_row` dropping the
carry-forward (`probe.rs:1527-1528`), `AgentBox`'s two field docs and `WriteStore::upsert_agent_box`'s
doc naming the single writer, and `.sqlx` regenerated.

Files: `crates/htui-store/src/pg/write.rs`, `crates/htui-core/src/store/{mem.rs,traits.rs,conformance.rs}`,
`crates/htui-core/src/model/agent.rs` (field docs), `crates/htui-agent/src/probe.rs`,
`crates/htui-store/tests/pg_criteria.rs`, `crates/htui-agent/tests/probe.rs`, `crates/htui-store/.sqlx/`.

**Serial after T38**, which touches the same four store files; both are one implementer's work, T38
first. It stays independent of T37 and T41.

### T41: the Settings quota column (independent of T37–T40; `ui/tabs/settings/agents.rs`)

Test first, snapshot-pinned: an `AgentSummary` whose `on_box.quota` carries two windows renders the
tightest one as a percentage with its reset; a per-token row renders session spend; a `None` renders
`—`; and the section states that `r` re-probes but cannot refresh quota. Then the eighth column and
the width redistribution.

Files: `crates/htui/src/ui/tabs/settings/agents.rs` and its test/snapshot module.

### T42: `R-AGT-8`'s predicate (independent of T40/T41; `model/quota.rs` — serial after T39)

Test first, one case per §7 skip rule plus the two that must *not* skip (null quota, and a window at
`0.99`). Then `available()`. No caller in this milestone beyond the tests: MOD-4 is the consumer,
and the plan says so rather than inventing a use.

Files: `crates/htui-core/src/model/quota.rs`.

### T46: criterion 6's dedup survives a flush (serial after T40 — same `record.rs`; D75)

Test first, at the recorder level and then in the suite:

1. `crates/htui-agent/tests/recorder.rs` — a script that announces a `tool_call` with a `diff`,
   **flushes** (a parked permission answered is the real-world trigger; a `usage` row or a kind change
   will also do it), then re-announces the *same* `tool_call` verbatim with the same path: exactly
   **one** `edit_proposal` row for that `(tool_call_id, path)`, updated in place, not two. Fails today
   because `flush()` clears the index.
2. `crates/htui-agent/src/conformance.rs` — `edit_proposal_deduped_per_call_and_path` (`:127-141`'s
   list) gains a flush between the two writes, so the transport-neutral case actually exercises the
   interleaving that broke live. The case name does not change; its script does.
3. The recorded `agy` fixture is the regression witness: replaying
   `crates/htui-agent/tests/fixtures/agy_acp_turn.jsonl` must produce **one** `edit_proposal` for the
   write, not three. T34's `acp_map.rs` case pins today's three as *mapper* output; this pins the
   *recorder*'s row count, which is where the rule lives.

Then the fix: the dedup index outlives the buffer. `flush()` stops clearing `edits`, which means the
stored value can no longer be a buffer index — it has to identify a row that is already persisted
(its `seq`), so an update after a flush is an update to a written row rather than to a slot that no
longer exists. Whatever shape that takes, two invariants hold: `seq` is still gapless, and a row is
never rewritten with a lower `seq` than one already flushed.

Files: `crates/htui-agent/src/record.rs`, `crates/htui-agent/src/conformance.rs`,
`crates/htui-agent/tests/recorder.rs`, plus any snapshot the row-count change moves.

### T47: the `default` column shows the whole model id (independent; `ui/tabs/settings/agents.rs`; D76)

Test first: a row whose `default_model` is `gemini-3.7-flash-high` renders that string **complete**
in the `default` cell at the 100-column render the section snapshots use, and the eight headers are
all still readable. Then rebalance the widths to pay for it, spending D76's priority order in
order — the full default model first, the quota window's reset time second (`62% to 09-08` without
the time is an acceptable loss), `on this box`'s slack third. Every existing
`settings__agents_*.snap` and `probe__agents_probed_missing.snap` moves with it and is re-accepted
with the diff reviewed line by line.

Constraint arithmetic to respect: the eight columns want ~107 columns and 98 are usable, so the task
is a packing decision with a stated ranking, not an arithmetic one. State the final widths and what
each column gives up in the commit message.

Files: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`, and the five
snapshots.

**T47 landed `4e3b3f0` and its packing was wrong; T48 repairs it.** T47 paid for `default`'s 12
columns with 6 from `quota` (19 → 13, dropping the reset's `%H:%M` — correct, priority 2) and 6 from
`on this box`'s slack, leaving that column at **11** while its own cells are `unauthenticated` (15),
`choose method` (13) and `downloading 12.0 MB` (19). `tests/auth.rs` went **0/5**, four of them
burning the 60 s deadline because the chooser cell drew `choose meth`, and the *app* drew
`unauthentic` — which defeats MOD-21's legible login. T47 reported this as unavoidable within D76 and
proposed changing the product's wording; **it was avoidable and no wording needed to change.**

**T48 (the correction, `28b0bdf`): `name` donates.** `name` was `Length(12)` holding at most `amp-acp` (7),
with a 4-character header — an allowance nothing used. Taking it to `Length(8)` returns exactly the 4
columns `on this box` needs to reach 15, so `unauthenticated` fits whole with `default` still 21 and
`quota` still 13. Every D76 priority survives and the vocabulary is untouched. The general lesson,
worth carrying into MOD-23's editor and MOD-12's caps section: **before trading a column's content
away, check which columns are holding width their content never uses** — the header is the floor, not
the current `Length`. `downloading 12.0 MB` (19) still clips, as it did at the old 17: pre-existing,
not new damage.

**And the slack is now gone.** After T48 the eight columns each sit at their own longest string —
`transport`/`models`/`enabled` at their headers, `billing` at `subscription`, `default` at the model
id, `quota` at `100% to 09-08`, `name` at `amp-acp`, `on this box` at `unauthenticated`. A **ninth**
column therefore costs a ranking decision like D76's rather than an adjustment, which is the fact
MOD-23's editor and MOD-12's caps section inherit. Verified live on this box: `auth` 5/5 and
`settings` 49/49 on the real tree at `28b0bdf`, not only in the implementer's worktree.
T48 also found that five *fixture* row names exceed 8 and now draw clipped in snapshots
(`needs-au`, `spend-on`, `unparsab`, `ready-of`, `demo-log`); they were deliberately **not** renamed,
because renaming would change snapshot content beyond widths, and the row lookup absorbs the
difference in one documented place instead. Test data only — no seeded or live agent name is affected.

### T43: criterion 7's ACP half, end to end (serial after T39)

Test: for an ACP session, `run_step.usage`'s `cost_micros` equals the last `cost_micros_total`
observed, against live Postgres — the second clause of criterion 7, which the existing conformance
case (`usage_deltas_sum_to_step_usage`) does not cover because it is transport-neutral.

Files: `crates/htui-store/tests/pg_criteria.rs`.

### T44: close-out (serial, last)

`README.md` gains the cap keys and their unit; the §11.14 answer table lands in this plan;
`HANDOFF.md` gains the phase-7 note (MOD-2 stays one open item until milestone 9, per
`.claude/rules/workflow-docs.md` lifecycle step 4); PRD milestone 6 → `complete` and 7 →
`complete`; the validator runs whole-repo.

## Validation

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
# Blocked by TOOL-3 on this box — fails in ring's build script (`lib.exe` not found), not in htui
# code. Re-confirmed 2026-09-10. Kept here so the next box with an MSVC toolchain runs it.
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace --all-features

# T34, explicitly (spawns the authenticated adapter and spends model tokens)
cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture

# D64's model list, re-run now that session/new succeeds
cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture
```

`-p htui --features testkit` is the correct invocation and the one `chat_live.rs:6` documents; the
milestone-6 plan's `--features demo,test-support` line for `chat_live_agy` was wrong — `htui`
declares no such features (`crates/htui/Cargo.toml:19-23`).

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| `agy_acp_server` reports no usage at all, so milestone 7's latch has one transport | Medium | Low | D65 makes that an *answer*, not a failure: the seed keeps `quota.source: "none"`, `agy`'s column reads `—`, and §7's "null means available" rule (D72) is exactly the case it was written for. `claude`'s blob is confirmed live from milestone 3 and is the latch's proof |
| A live turn spends real subscription tokens on the maintainer's account | High | Low | One short turn, `#[ignore]` by default, run deliberately. The ToS risk is ANA-4 §10 risk 8, accepted at milestone 6's CONFIRM (D63) and unchanged here |
| The permission probe in T34 edits a real file | Medium | Medium | The turn works inside a scratch directory created by the test, and the test answers the permission itself; `SessionSpec.cwd` is the bound, and the path guard `682a423` fixed is the backstop |
| A cap that is wrong by a factor of a million cancels every session | Low | High | D70 fixes the unit as micros, the reader rejects a negative, and an absent key means unbounded. The unit is stated in `README.md` and in the test names, and a cap below one `usage` row's delta is the explicit "cancels on the first row" test case |
| The cap's client-side estimate diverges from the actual bill | Medium | Medium | ANA-4 §7 already frames caps as guard rails and never as billing statements; the `error` message says estimate. `--max-budget-usd`, the server-side second cap, is the CLI transport's and therefore milestone 8's |
| The latch races a concurrent re-probe of the same row | ~~Low~~ **certain, by construction** | Medium | **Closed by D74**, not accepted: `upsert_agent_box` can no longer write `quota`/`quota_at`, so the probe's read-modify-write cannot discard a latch. D67's narrow setter already protected `probe` in the other direction, and D60's runtime-wide claim only ever serialised probes against each other — never a probe against a latch |
| A vendor quota blob carries a credential-shaped value | Low | High | It is a payload and goes through the scrubber like every other (`R-SEC-3`, fail-closed); the residue path refuses the write (`record.rs:749-754`) rather than persisting it |

## Verified claims

Checked against the tree on `main` at 2026-09-10, before CONFIRM. Evidence is a file:line in this
tree, a command run on this box, or a document section — never recall.

| Claim | Verdict | Evidence |
|---|---|---|
| `agent_box.quota` and `quota_at` already exist, nullable, no default | true | `crates/htui-store/migrations/0001_init.sql:119-120` |
| `0002_agent_probe.sql` adds only `probe` to `agent_box` | true | `migrations/0002_agent_probe.sql:25` |
| No migration is needed for this milestone, so MOD-4's `0003` stays unheld | true | the two rows above + `per_token_cap_*` being JSON keys, `0001_init.sql:150-151` |
| `per_token_cap_run`/`per_token_cap_batch` are `project.settings` JSON keys, not columns, and are never seeded | true | `0001_init.sql:150-151`; the only other hits are the `app_setting.key` comment `0001_init.sql:558` and a doc comment `htui-core/src/model/hierarchy.rs:43` |
| The 12 seeded `app_setting` keys include no cap key | true | `htui-store/src/pg/mod.rs:47-50` (2) + `migrations/0002_agent_probe.sql:68-79` (10) |
| Nothing in the tree writes `agent_box.quota` today | true | the only writer of the row is `WriteStore::upsert_agent_box`, `traits.rs:117` / `pg/write.rs:418-436`; `probe.rs:1504,1528` only carries the old value forward |
| The ACP mapper never reads `_meta` | true | `acp/map.rs:74-107`; `QuotaSource::AcpMetaRateLimit` (`launch.rs:388-389`) has no reader |
| The rate-limit blob does arrive under `_meta["_claude/rateLimit"]`, on a later `usage_update` than the first | true | milestone 3's live recording, `HANDOFF.md` phase-3 note |
| `UsageEvent` carries `cost_micros` (delta) and `cost_micros_total` (cumulative), both USD-gated | true | `acp/map.rs:92-103`, `event.rs:328-362` |
| `UsageTotals` sums five fixed keys and ignores unknown ones, so a new `UsageEvent` key cannot enter a sum | true | `htui-core/src/model/usage.rs:24-38`, `:48-54`, `:90` |
| Adding `quota: Option<Value>` to `UsageEvent` keeps every recorded fixture and persisted row deserializable, and serializes away when `None` | true | compile probe run on this box 2026-09-10 (serde 1 derive, `cargo run --offline`): `Option<Value>` with **no** `#[serde(default)]` deserializes from `{"a":1}` as `None`, and with `skip_serializing_if = "Option::is_none"` re-serializes to `{"a":1}`. The repo's own convention still spells `#[serde(default)]` on such fields (`probe.rs:1087` `credential`, `model/agent.rs:94` `probe`), and T37 follows the convention rather than the minimum |
| `Recorder` holds no `AgentSession` and cannot call `cancel()` — ANA-4 §7's stated mechanism is not implementable as written | true | `record.rs:211-243`: 20 fields, none a session. The file imports `AgentSession` (`record.rs:69`) **only** for `pump`'s parameter, and the type appears nowhere else in it. `AgentSession::cancel` is declared `driver.rs:425`, impls `acp/mod.rs:688` and `fake.rs:404`; the only live caller is `agent_worker.rs:2352` |
| `pump` holds both the session and the recorder, so it can | true, but **incomplete** — corrected by the blueprint (P-1) | `record.rs:965-982` holds both, and it is what the conformance suites and `tests/recorder.rs` drive (18 call sites). **Production never calls it**: `grep -rn 'pump(' crates/htui/src/` returns nothing; `run_chat` drives `run_turn` (`agent_worker.rs:2189-2200`, `:2300-2434`), whose own doc calls itself "`pump`'s shape with one difference" (`:2290-2295`) because `next_event` refuses while a permission is parked. Resolution: the cancel-and-close sequence is one shared `enforce_breach`, called from both loops, and both are tested |
| The two new agent conformance cases go in `htui-core/src/store/conformance.rs` | **false** — corrected (P-2) | That is the *store* suite: 20 cases (`store/conformance.rs:24-45`), pinned by `pg_conformance.rs:19`. The 13-case list is `htui-agent/src/conformance.rs:127-141`. Three cases across two suites: store 20 → 21 (T38), agent 13 → 15 (T39, T40) |
| Criterion 7's ACP proof can live in `htui-store/tests/pg_criteria.rs` | **false** — corrected (P-3) | `grep htui-agent crates/htui-store/Cargo.toml` returns nothing (no build **or** dev dependency), so that crate cannot construct a `Mapper` or a `Recorder`. The proof moves to a new `crates/htui/tests/chat_usage_pg.rs` |
| `QuotaSource` can be referenced from `htui-core` as it stands | **false** — corrected (P-6) | It is a `wire_enum!` in `htui-agent` (`launch.rs:384-398`) and `grep -rn QuotaSource crates/htui-core/src/` is empty. It **moves** to `htui_core::model::quota` as a `str_enum!` with the same four wire strings, and `htui_agent::launch` re-exports it so `lib.rs:137` and `tests/launch.rs` keep resolving |
| T34's turn is bounded by a `cwd` carried on the request | **false** — corrected (P-7) | `start()` sets `cwd = std::env::current_dir()` (`agent_worker.rs:1081`); nothing on `ChatStart` carries a directory. The live test sets the process working directory to a `tempfile::tempdir()` behind a `Drop` guard — `set_current_dir` is safe, so `forbid(unsafe_code)` is untouched |
| `cap_exceeded` exists only as a doc-comment example; no cap code exists | true | `event.rs:367`; zero hits for `per_token_cap`, `max_budget_usd` in `crates/` |
| `StopReason::Cancelled` exists as a wire value | true | `event.rs:77`, `:93` |
| `ErrorEvent` is `{ code, message }` | true | `event.rs:366-371` |
| `agent.settings.quota.source` is a declared four-value enum | true | `launch.rs:379-398` (`QuotaSettings`, `QuotaSource`) |
| `AgentSummary` already carries the `agent_box` row, so the quota column needs no new read path | true | `htui-core/src/model/agent.rs:165-170`; `AgentBox.quota` at `:79`, `quota_at` at `:81` |
| The Settings agents table has exactly seven columns and no quota column | true | `crates/htui/src/ui/tabs/settings/agents.rs:973-980` |
| `r` maps to `ProbeAgents` in that section | true | `ui/tabs/settings/agents.rs:1101` |
| The Chat tab renders no usage or cost, and never reads `DriverCaps.usage` | true | `ui/tabs/chat/mod.rs:323-336`, `:525-557`; `DriverCaps.usage` at `driver.rs:332-333` |
| `RunStepSummary` has no `usage` field, and that projection is MOD-4's | true | `htui-core/src/model/run.rs:264-287`; ANA-4 §7 ("Nothing renders it yet"), ANA-4:1279-1280 |
| The batch cap is MOD-12's, not this milestone's | true | `docs/ANA-4.md`:1283 |
| `ChatStart` carries `project_id`, so the caps are reachable at session start | true | `crates/htui/src/store_worker.rs:96-105` |
| Registry-style reads are inherent per store with a three-arm `Backend` dispatch, not trait methods | true | `htui-core/src/store/traits.rs:11-14`; `backend.rs:292`, `mem.rs:223`, `pg/read.rs:538`, `cache/read.rs:730` |
| The cache mirror carries `project.settings`, so D70's reader works offline | true | `cache_migrations/0001_mirror.sql:57-61` |
| The cache mirror has no `agent_box` table, so the latch cannot be mirrored | true | `cache_migrations/0002_agent_mirror.sql:9-11` states the deliberate omission; the file is 19 lines and its only `CREATE TABLE` is `agent` (`:14`), and `grep agent_box cache_migrations/*.sql` returns only that comment |
| `Writer::Buffered` already refuses registry writes with `REGISTRY_ON_SERVER_ONLY` | true | `agent_worker.rs:1521-1531`, `:702-706` |
| `conformance::CASES` is 13 entries and already covers usage summing and cancel-with-parked-permissions | true | `conformance.rs:127-141`; `:1092` and `:810` |
| `htui`'s test feature is `testkit`; `demo`/`test-support` do not exist on that crate | true | `crates/htui/Cargo.toml:19-23`; `chat_live.rs:6` documents the invocation — the milestone-6 plan's T34 command line was wrong |
| `chat_live.rs` is a usable model for T34 (production runtime, survivor check, row assertions) | true | `crates/htui/tests/chat_live.rs:29-167` |
| `agy_live.rs` has four cases and case 3 is the `session/new` config-options probe | true | `crates/htui-agent/tests/agy_live.rs:347,490,592,768` |
| The box is authenticated for `agy_acp_server`, so T34 is unblocked | true | `~/.gemini/antigravity-acp/acp_token.json` exists (checked 2026-09-10); MOD-21's live login, `docs/decisions/mod/mod-21.md` |
| Dev Postgres is up on 5439 | true | `docker ps` → `htui-postgres 0.0.0.0:5439->5432/tcp` |
| The baseline suite is green before this work | true | `cargo test --workspace --all-features` exit 0 on `main`, 2026-09-10 |
| `probe::agent_box_row` carries `quota`/`quota_at` forward from a previously read row, and `upsert_agent_box`'s `SET` list writes both — so a re-probe discards any latch that landed in between (D74's target) | true | `probe.rs:1527-1528` carries them and `probe.rs:1504-1505` says "the probe owns neither"; `pg/write.rs:426-427` writes `quota = EXCLUDED.quota, quota_at = EXCLUDED.quota_at`; `mem.rs:840-842` replaces the whole row (`*stored = row.clone()`) |
| Task file sets: T37 (`event.rs`, `acp/map.rs`), T38 (store crates), T41 (`settings/agents.rs`) are pairwise disjoint; T45 shares T38's four store files and runs serially after it; T39/T40 share `record.rs`; T42 shares `quota.rs` with T39; T34 touches only tests + seed unless D62 fires | true | the per-task file lists above, intersected — T39→T40 and T39→T42 run serial, T37/T38/T41 may run parallel, T34 runs first by D65 |

## Acceptance

1. A live `agy` turn streams through the production runtime with the same structural floor
   `chat_live.rs` asserts, and no `agy_acp_server` survives it (criterion 11's second binary).
2. The three outstanding ANA-4 §11.14 `agy` items are answered in writing, in this plan and in the
   MOD-2 decision document; a demanded twelfth `DriverEvent` variant would be reported, not added.
3. `agent_box.quota` is latched from the wire for a row that declares a source, in §7's document
   shape, with `quota_at` set — and `probe` is byte-identical across the write.
3a. The two quota columns are **single-writer** (D74): `upsert_agent_box` can neither set nor clear
   them, on the insert path or the conflict path, and a re-probe running beside a chat cannot
   discard a latch. Proven at SQL level, not just through the trait.
4. An offline chat neither latches nor fails, and says why (D68).
5. A per-run cap breach cancels the session within one event and leaves
   `error{code:"cap_exceeded"}` then `done{stop_reason:"cancelled"}` as the step's last two rows
   (ANA-4 §11 criterion 8).
6. `run_step.usage` equals the sum of the step's `usage` rows and, for an ACP session, its
   `cost_micros` equals the last `cost_micros_total` observed (criterion 7, both clauses).
7. `R-AGT-8`'s predicate exists and is tested per skip rule, with null quota treated as available.
8. Settings shows quota per agent for this box and states that `r` cannot refresh it (`R-TUI-8`),
   and the `default` column shows a model id **in full** rather than truncated (D76) — the id is
   ANA-4 §4.4's selection coordinate, so a clipped one is wrong data, not a cosmetic.
8a. ANA-4 §11 **criterion 6 holds across a flush** (D75): one `agy` file write leaves exactly one
   `edit_proposal` row per `(tool_call_id, path)`, proven against the recorded live fixture and by a
   conformance case whose script now flushes between the two writes.
9. Nothing added is keyed on an agent name (`R-AGT-5`); no migration was added.
10. Full suite green on Linux with Postgres live. **The Windows lint target cannot be claimed and is
    not an acceptance condition**: `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent` dies
    in `ring`'s build script (`error occurred in cc-rs: failed to find tool "lib.exe"`) before any
    `htui` code compiles — `HANDOFF.md` **TOOL-3**, re-confirmed on this box on 2026-09-10, and the
    same state MOD-20 and MOD-21 recorded (`docs/decisions/mod/mod-21.md:145`, MOD-21 D22). This
    milestone adds no `cfg`, no platform code and no process code — a serde field, a `&str` const, a
    JSON read, a pure model module, two SQL statements and a recorder change — so nothing here is
    platform-sensitive; but "green" would be a claim about a command that did not run. **MOD-16
    inherits it**, as it does every other Windows fact.
11. `rust-reviewer` gate clear (`.claude/workflow-config.json`).

## The ANA-4 §11.14 answers (T34, live on this box 2026-09-10, `b094d96`)

`antigravity-acp` 1.1.1, three turns, **run twice with both runs agreeing**. Transcript recorded at
`crates/htui-agent/tests/fixtures/agy_acp_turn.jsonl`; the live test is
`crates/htui/tests/chat_live_agy.rs` (passed twice, 12.4 s and 38.8 s, no surviving
`agy_acp_server` either time). T44 copies this table into `docs/decisions/mod/mod-2.md`.

| §11.14 item | Answer |
|---|---|
| Does `agy_acp_server` emit `usage_update`, and in what field? (`:1384`) | **It emits none at all** — zero `Usage` frames across three turns including a tool call and a file write. Not partial, not context-only. The nine recorded `session/update` lines are `available_commands_update`, `agent_message_chunk` ×2, `tool_call` ×3, `tool_call_update` ×3; no `cost`, no `used`/`size`, no `_meta` anywhere. ANA-4 §7's "Unverified" row for `agy` resolves to **nothing**, so `settings.quota.source` stays `"none"`, `agy`'s quota column reads `—` by design, and milestone 7's latch has exactly one transport — D65's explicit "that is an answer, not a failure" branch |
| Does it issue `session/request_permission` in `default` mode, with what option ids and kinds? (`:1385-1386`) | **Yes, for the write and not for the read.** `session/new` confirms `modes.currentModeId == "default"`. Turn 2's read (`tool_kind: Read`) produced no request; turn 3's write produced exactly one, offering `{"id":"allow","label":"Allow","kind":"allow_once"}` and `{"id":"deny","label":"Deny","kind":"reject_once"}` — **no** `allow_always`, **no** `reject_always`. `htui`'s own policy was `ask`, so nothing auto-answered: the gating is entirely the adapter's. The `tool_call` is announced `status: "pending"` **with its diff before** the request, so the diff is on screen while the user decides |
| Standard `tool_call` + `diff`, or a vendor shape? (`:1387-1388`) | **Standard**: `tool_call` with `kind: "edit"` and a `diff` content block, landing in the typed variants rather than `other`. The vendor's own names ride *inside* schema fields (`rawInput: { code_content, target_file }`, a `_meta: { kind: "add" }` on the diff block), so the existing mapper reads all of it. **No mapper change was demanded** — eleven `DriverEvent` variants unchanged, no new `EventKind`, nothing keyed on `"agy"` (D62 held) |

**D64 resolved, opposite to milestone 6's fallback.** `session/new` returns `configOptions` with
`id: "model"` (category `model`, type `select`, `currentValue: "gemini-3.7-flash-high"`) and eleven
values, plus a second entry `id: "mode"` (`default` / `auto_edit` / `yolo` — the permission-mode
coordinate). `crates/htui-core/seeds/agent_agy.json` now carries the eleven ids,
`default_model: "gemini-3.7-flash-high"` and `settings.acp.model_config_id: "model"`, and the live
re-run confirms the selection is **accepted** (no `model_unavailable` row) — so ANA-4 §4.4's
selection-by-config-option-id is exercised end to end for the first time. The seed-pinning test in
`crates/htui-core/src/model/agent.rs` now asserts the two rows separately (`claude` stays empty).

**Process note, recorded because it cost real rework.** D65 said T34 runs "first, alone"; it did not.
T41 was launched in parallel on the strength of a file-set intersection that **this plan got wrong**
— seeding `agy`'s `models`/`default_model` moves four snapshots, two of them
(`settings__agents_demo`, `settings__agents_unknown_row`) inside T41's set. The coupling is
seed → rendered table → snapshot, which a file-set intersection over *source* files does not
reveal. Both agents recovered (T41's content is correct in `142beb1`; T34 rebased its snapshot work
onto it), but the lesson for the remaining tasks is that **snapshot files are part of a task's file
set even when no source file overlaps**.

## Close-out

Phase-7 note appended to `HANDOFF.md`'s MOD-2 entry per `.claude/rules/workflow-docs.md` lifecycle
step 4 (MOD-2 stays one open item until milestone 9); PRD milestone rows 6 and 7 → `complete`;
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` green whole-repo; commits
reported. Push only when agreed.
