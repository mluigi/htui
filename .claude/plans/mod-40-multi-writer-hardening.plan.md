# Plan: MOD-40 — Multi-writer store hardening

**Status: CONFIRMED by the maintainer 2026-09-26, OQ-1 and OQ-2 as recommended.**

**Source**: `.claude/prds/mod-40-multi-writer-hardening.prd.md`, all four milestones, with its gate
decisions PRD D1-D3 (maintainer, 2026-09-26, "all recomm"). Design: `docs/ANA-16.md` §6.1, gaps
C1-C8.

**Requirements**: `R-ID-3`, `R-HIS-1`, `R-STO-5` (amended 2026-09-26: a headless process never
migrates; it refuses and reports).

**Complexity**: Large in breadth, small in depth. No migration. One new `StoreError` variant, one
new trait type (`StepFence`), signature changes on eight `WriteStore` methods, two new `PgStore`
inherent methods, one moved trait (`Clock`).

**Routing**: `/handoff-run MOD-40`, PRD path, ultracode recommended for implement (C4). Reviewer:
`rust-reviewer` (`.claude/workflow-config.json:2`). There is no `.claude/agents/`, so the reviewer
runs as a general-purpose agent under that brief.

**Numbering**: decisions **D1…** are this plan's; the PRD's are cited as **PRD D1-D3**. Tasks
**T1…**, risks **R-1…**, open questions **OQ-1…**.

**Graphify / Gortex note**: `graphify-out/` does not exist and Gortex is not reachable in this
session. Tree facts were read with `grep`/`sed` at `edf19c0` and carry a `file:line` (paths relative
to `crates/`).

---

## Open questions for the maintainer (read these first)

- **OQ-1 — Fence only the three named writes?** Recommended: yes, `append_events`,
  `set_step_usage` and `finish_step` (PRD D1). `set_step_prompt` and `upsert_step_tree` are also
  step writes, but they run before the session (`engine.rs:3106`, `:3144`) and before
  `finish_step` (`:3707`, `:3790`), so a stale holder that wakes mid-session meets a fenced
  recorder flush first and its walk stops there. `interrupt_step` is the sweep's own write
  (`engine.rs:9280`) and must not be fenced. The rest go into a note on MOD-41. Alternative: fence
  `set_step_prompt` and `upsert_step_tree` too (two more signatures). The MOD-41 note also names
  `record_commits` and the sink's output write: unfenced step writes on the settle tail, which a
  holder that wakes **after** `done` makes before it reaches its fenced `finish_step` (blueprint
  F-7).
- **OQ-2 — The heartbeat fences on local elapsed time, not on the returned stamp.** PRD D2 says
  Postgres "returns what it wrote". Recommended refinement: the lease methods take a TTL, Postgres
  stamps `clock_timestamp() + ttl`, and the heartbeat's self-fence is `send time + ttl - margin` on
  the **local** clock. A duration measured on one clock is skew-free, while comparing a
  database-stamped instant with the local clock reintroduces the skew C2 removes. Nothing then needs
  the stamp back, so the lease methods keep their `bool` / `Claim` / `Vec<Run>` answers.
  Alternative: return the stamp as PRD D2 literally says, and fence on it (skew-sensitive).

## Summary

Four milestones, one branch, no migration. M1 fences step writes by lease owner and makes a short
fresh transcript insert loud (C1, C8). M2 guards quota order, puts a CAS on agent edits and pins the
box-settings CAS (C3, C6, C7). M3 adds a box heartbeat and a headless connect that refuses skew
(C4, C5). M4 moves lease time into Postgres (C2). Each milestone ends green and is committed; the
branch is pushed only on the maintainer's ok.

## Design decisions (settled here, not in code review)

- **D1 — `StepFence`** (PRD D1). In `htui-core/src/store/traits.rs` next to `CasOutcome`:
  `pub enum StepFence { Lease(Uuid), Unleased }`, `Copy`, documented. The three writes take it
  first after `&self`: `append_events(fence, events)`, `set_step_usage(fence, step, usage, digest)`,
  `finish_step(fence, step, outcome)`.
  - **Postgres**: the predicate joins through `run` in the same statement.
    `append_events` becomes `INSERT … SELECT … FROM jsonb_to_recordset($1) e JOIN run_step s ON
    s.id = e.run_step_id JOIN run r ON r.id = s.run_id WHERE r.lease_owner IS NOT DISTINCT FROM $2
    ON CONFLICT … DO NOTHING`, with `$2` NULL for `Unleased`. The two updates add
    `AND EXISTS (SELECT 1 FROM run r WHERE r.id = run_step.run_id AND r.lease_owner IS NOT DISTINCT
    FROM $n)`. One `IS NOT DISTINCT FROM` covers both variants.
  - **A miss** (zero rows) is told apart by one follow-up read, the `cas_miss` shape
    (`write.rs:81`): no step keeps today's answer (`Constraint` "references no run_step" from
    `append_events`, as MemStore says at `mem.rs:1436-1443`; `NotFound` from the two updates), and
    a step present under another owner is `StoreError::Fenced { step }`. `append_events` runs the
    follow-up read only when fewer rows landed than were offered: a batch whose rows all already
    exist under a matching fence stays a short `Ok` (replay).
  - **MemStore**: `lease_owners.get(&run_id).copied()` (`mem.rs:181`) compared with the fence's
    `Option<Uuid>`; checked before any write, like its step-existence check (`mem.rs:1436-1443`).
  - **`StoreError::Fenced { step: StepId }`**, `#[error("run_step {step} is not writable under this
    lease")]`. No exhaustive `match` on `StoreError` exists outside tests (grep, below), so the
    variant is additive.
- **D2 — The recorder carries the fence.** `Recorder::with_fence(StepFence)` is a builder like
  `with_quota_latch`/`with_run_cap` (`record.rs`), default `Unleased`. The engine's
  `open_recorder` (`engine.rs:5265-5293`) calls `.with_fence(StepFence::Lease(self.parts.owner))`;
  chats (`agent_worker.rs:3102`, `:3107`) keep the default. A forgotten fence on an engine recorder
  fails loudly (`Fenced`), never silently. The five engine `finish_step` sites pass
  `StepFence::Lease(self.parts.owner)`.
- **D3 — C8: replay and fresh rows are separate calls.** `flush` (`record.rs:1018-1052`) first
  re-offers `unflushed` alone (a short count is fine: that is the replay), then appends the fresh
  batch; a fresh batch that inserts fewer rows than it offered is
  `RecordError::Store(StoreError::Constraint("session_event seq … already held: a second writer on
  this step"))` and is **not** put back in `unflushed` (a retry would be a replay and would hide
  it). A refused fresh batch (store error) is re-queued as today. No new `RecordError` variant, so
  `From<RecordError> for DriverError` (`record.rs:261-269`) is unchanged. The stale "offline
  buffer" docs (`traits.rs:285-295`, `write.rs:937-939`, `writer.rs:15-24`, `:85-106`) are
  rewritten to name the recorder's retry.
- **D4 — C3: an older quota is a no-op, not an error.** `set_agent_box_quota` returns
  `Result<bool>`: `true` written, `false` an equal-or-newer `quota_at` is stored; `NotFound` only
  when the row is absent (one follow-up read). Predicate `AND (quota_at IS NULL OR quota_at <= $4)`
  (ANA-16's; `<=` keeps a same-instant rewrite idempotent). `latch_quota` ignores the bool.
- **D5 — C6: `upsert_agent` becomes a CAS on `updated_at`.** Signature `upsert_agent(&Agent,
  expected: Option<DateTime<Utc>>) -> Result<CasOutcome<Agent>>`, the `set_setting` App-rung shape
  (`traits.rs:832-851`): `None` = "I expect no row" (insert; `Stale` with the stored row if one
  exists), `Some(t)` = update `WHERE id = $1 AND updated_at = $t`, `Stale` on a miss, `NotFound`
  if absent. Test callers that seed pass `None`; the ones that edit an existing agent go through a
  test helper that reads the row's `updated_at` first.
- **D6 — C7 is a pin.** `edit_box` is the only `box.settings` writer and is already a CAS
  (`write.rs:1350-1354`). A trait-doc invariant on `edit_box` ("every writer of `box.settings` is a
  compare-and-set") and a conformance case `a_stale_box_edit_diverges` if none exists.
- **D7 — C4: `PgStore::touch_box`.** Inherent on `PgStore` (like `register_box`, `pg/mod.rs`),
  `UPDATE box SET last_seen_at = clock_timestamp() WHERE id = $1`, answering `Ok(bool)`. The store
  worker gains a `box_beat` interval (`BOX_HEARTBEAT = 60 s`, `interval_at` + `Delay`, the
  `sweep_ticker` shape `store_worker.rs:1842-1846`) whose arm runs only while online and spawns the
  touch without awaiting it in the loop. No reader is added (PRD out of scope).
- **D8 — C5: `PgStore::connect_headless`.** `connect_headless(dsn, identity, timeout) ->
  Result<PgStore, HeadlessError>`, sharing `connect_with`'s pool and `schema_state`
  (`pg/mod.rs:168-193`, `:584-627`). `HeadlessError { Store(StoreError), MigrationsPending(usize),
  BelowTarget { ours: String, target: String } }`, `thiserror`, exported from `htui_store`. It never
  calls `MIGRATOR.run`. `concepts.rs:52-62` moves onto it (its bail text is kept).
- **D9 — Target version** (PRD D3). `app_setting` key `htui_target_version`, a JSON string. Written
  by `apply_migrations` (`pg/mod.rs:234-237`) after `bootstrap`, in one transaction: `SELECT … FOR
  UPDATE`, compare with `semver` (workspace dep, `Cargo.toml:83`), write only if ours is greater or
  the row is absent. Read by `connect_headless` (refuse below) and by `connect_with`
  (`Connected.below_target: Option<String>`, the target when ours is lower). The TUI shows it once
  as a status-line notice through the existing `StoreState` reply; exact field named in T6 after the
  blueprint reads the shell. Unknown `app_setting` keys are ignored by every reader (keyed lookups
  only: `connect.rs:616-637`, `prompt/settings.rs:647`, `:696`).
- **D10 — C2: lease methods take a TTL** (PRD D2, OQ-2). `claim_run(run, box, owner, ttl)`,
  `refresh_lease(run, owner, ttl)`, `adopt_runs(box, owner, ttl)`, `take_lease(run, box, owner,
  ttl)`, `release_lease(run, owner)`. Postgres stamps `clock_timestamp()` for `started_at`,
  `lease_expires_at` and every expiry comparison; MemStore reads its clock (D11). The heartbeat
  (`recover.rs:114-160`) keeps its loop and fence arithmetic on the local clock, with `written` the
  local send time + ttl.
- **D11 — `Clock` moves to `htui-core`.** `htui-core/src/clock.rs`: `Clock`, `SystemClock`, and
  `TestClock` (from `htui-orch/src/fake.rs:791-841`) behind the existing test-support feature or
  `cfg(any(test, feature = …))` (blueprint picks the gate). `htui-orch` re-exports both at their old
  paths. `MemStore::with_clock(Arc<dyn Clock>)`; the default is `SystemClock`, and every
  `Utc::now()` in the wrappers (`mem.rs:5926-5973`) reads it.

## Patterns to Mirror

- CAS miss → one follow-up read: `cas_miss` (`write.rs:81`), `transition`'s `SELECT 1`.
- CAS outcome type: `CasOutcome<T>` (`traits.rs:1974-1979`), `set_setting` (`:851`).
- Recorder builders: `with_quota_latch`, `with_run_cap` (`record.rs`, used at `engine.rs:5281`).
- Lease CAS on owner in MemStore: `refresh_lease` (`mem.rs:3983-3997`).
- SQL time: `register_box`'s `last_seen_at = clock_timestamp()` (`pg/mod.rs:460`).
- Tickers: `sweep_ticker` (`store_worker.rs:1842-1846`).

## Tasks

**M1: T1 → T2. M2: T3 → T4. M3: T5 → T6. M4: T7 → T8.** Milestones run in order; each task is
serial with its neighbour because every pair shares `traits.rs`/`mem.rs`/`pg/write.rs` or
`pg/mod.rs`. Ultracode applies inside T4 and T8 (mechanical call-site sweeps over disjoint test
files), not across tasks.

| Task | Files (complete list) | Parallel |
|---|---|---|
| T1 | `htui-core/src/store/{traits.rs, error.rs, mem.rs, conformance.rs, mod.rs}`, `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `htui-store/.sqlx/`, `htui-agent/src/conformance.rs` (`UsageSpy`), `htui-agent/tests/recorder.rs` (`SpyStore`), `htui-store/tests/pg_criteria.rs`, `htui/tests/{runs_pg.rs, chat.rs, chat_usage_pg.rs}` (call sites) | first |
| T2 | `htui-agent/src/record.rs`, `htui-orch/src/engine.rs`, `htui-orch/src/conformance.rs`, `htui/src/agent_worker.rs` (only if a chat test needs the fence named) | after T1 |
| T3 | `htui-core/src/store/{traits.rs, mem.rs, conformance.rs}`, `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `htui-store/.sqlx/`, `htui-agent/src/{conformance.rs, record.rs}`, `htui-agent/tests/recorder.rs`, `htui-store/tests/pg_criteria.rs`, `htui/src/agent_worker.rs` (test at `:5101`), `htui-orch/src/conformance.rs` (`:2469`) | after T2 |
| T4 | T3's store files, plus every `upsert_agent` caller: `htui/src/{agent_worker.rs, run_worker.rs}` (tests), `htui/tests/{backlog.rs, box_probe.rs, box_probe_pg.rs, auth.rs, chat_live_cli.rs}` | after T3 |
| T5 | `htui-store/src/pg/mod.rs`, `htui/src/store_worker.rs`, `htui-store/tests/pg_criteria.rs`, `htui-store/.sqlx/` | after T4 |
| T6 | `htui-store/src/pg/mod.rs`, `htui-store/src/lib.rs`, `htui-store/src/connect.rs` (carry `below_target`), `htui/src/concepts.rs`, `htui/src/store_worker.rs` + the shell's status notice (files named by the blueprint), `htui-store/tests/{migrations.rs, connect.rs}`, `htui-store/.sqlx/` | after T5 |
| T7 | `htui-core/src/{clock.rs (new), lib.rs}`, `htui-core/src/store/mem.rs`, `htui-orch/src/{isolate.rs, fake.rs, lib.rs}` | after T6 |
| T8 | `htui-core/src/store/{traits.rs, mem.rs, conformance.rs}`, `htui-store/src/pg/write.rs`, `htui-store/src/writer.rs`, `htui-store/.sqlx/`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs`, `htui-orch/src/{engine.rs, recover.rs, conformance.rs}`, `htui/src/run_worker.rs`, `htui-store/tests/pg_criteria.rs` | after T7 |

Every implementer prompt carries these rules:
- PRD D1-D3, this plan's D1-D11 and the maintainer's OQ answers win over prose.
- Read the tree with grep and read (no `graphify-out/`, no Gortex).
- No test is skipped or loosened; a moved pin names its reason in the assertion message.
- Every new `pub` item has a doc comment and a `Debug`.
- Red tests committed first, then green.
- Gate: `RUST_BACKTRACE=0 cargo test -p <crate> --all-features -- --test-threads=2`, with the
  Postgres env for `htui-store` and `htui`.

### T1: step fence in the store (D1)
- **Tests first** (store conformance, both stores):
  `a_stale_owner_writes_nothing_to_the_step` (claim as A, `adopt_runs`/`take_lease` as B, then A's
  three writes → `Fenced`, rows and settle columns unchanged), `the_new_owner_writes_the_step`,
  `an_unleased_fence_writes_a_chat_step`, `an_unleased_fence_is_refused_on_a_leased_run`,
  `a_replayed_batch_under_the_right_fence_is_ok_zero`, `a_missing_step_keeps_its_old_error_not_fenced`.
  Existing `append_events_idempotent_and_ordered` (`conformance.rs:1305`),
  `set_step_usage_writes_usage_and_digest` (`:1393`), `finish_step_records_the_settle` (`:7468`)
  keep their assertions with the fence added.
- **Action**: D1. `cargo sqlx prepare` for the three changed queries.
- **Validate**: `htui-core`, `htui-store` gates; `cargo build --workspace --all-features
  --all-targets`.

### T2: recorder and engine (D2, D3)
- **Tests first**: `record.rs` — `a_fresh_batch_that_lands_short_is_an_error_and_not_requeued`,
  `a_replayed_batch_may_land_short`, `the_fence_rides_every_write`; `htui-orch` conformance —
  `a_suspended_walk_cannot_write_after_adoption` (walk A stalls, B adopts, A's recorder flush and
  `finish_step` are `Fenced`, B's step rows intact); chats: the existing `htui/tests/chat.rs` and
  `chat_usage_pg.rs` pass unchanged (unleased default).
- **Action**: D2, D3; doc fixes.
- **Validate**: `htui-agent`, `htui-orch`, `htui` gates.

### T3: quota order and the C7 pin (D4, D6)
- **Tests first**: `an_older_quota_is_a_no_op`, `an_equal_quota_at_rewrites_idempotently`,
  `a_missing_agent_box_is_not_found`, `a_stale_box_edit_diverges` (if absent).
- **Action**: D4, D6.

### T4: agent CAS (D5)
- **Tests first**: `upsert_agent_with_none_inserts_and_is_stale_when_present`,
  `upsert_agent_with_a_spent_token_is_stale_and_writes_nothing`,
  `upsert_agent_with_the_current_token_applies`, on both stores.
- **Action**: D5; sweep the call sites (65 `upsert_agent(` hits workspace-wide).

### T5: box heartbeat (D7)
- **Tests first**: `pg_criteria` `touch_box_advances_last_seen_at`, `touch_box_on_an_unknown_box_is_false`;
  a store-worker test that the arm is gated on a writer.
- **Action**: D7.

### T6: headless connect and target version (D8, D9)
- **Tests first** (`migrations.rs`/`connect.rs`): `a_headless_connect_never_migrates`
  (`Pending(n)` → `MigrationsPending(n)`, `_sqlx_migrations` unchanged),
  `a_headless_connect_refuses_a_newer_schema`, `applying_migrations_raises_the_target_and_never_lowers_it`,
  `a_headless_connect_below_the_target_refuses`, `a_tui_connect_below_the_target_reports_it`.
- **Action**: D8, D9; `concepts.rs` onto `connect_headless`.

### T7: clock into `htui-core` (D11)
- **Tests first**: `a_mem_store_reads_its_clock` (a `TestClock` advanced moves `updated_at`).
- **Action**: D11; no behaviour change, every existing test green.

### T8: database time for leases (D10)
- **Tests first**: `pg_criteria` `lease_times_are_the_databases` (a caller whose clock is a day off
  still gets `lease_expires_at` within seconds of `clock_timestamp()`); conformance cases that
  expired a lease by passing a future `now` rewritten to expire it by the store's clock (MemStore:
  `TestClock::advance`; Postgres: `ttl = 0`); `recover.rs` heartbeat tests on local elapsed time.
- **Action**: D10; every lease case in `htui-core/src/store/conformance.rs` (`:4334`, `:4530`,
  `:4763`, `:4932`, `:5088`, `:5233`, `:5392`) and `htui-orch/src/conformance.rs` (`:3914-3947`,
  `:4645-4658`, `:5337`) re-expressed, meaning unchanged.

## Test plan

Store behaviour is pinned by the conformance suite on MemStore and PgStore (both), plus
`pg_criteria` for Postgres-only facts (SQL time, the heartbeat, concurrency). The recorder is tested
over `SpyStore`/MemStore; the engine over the orch conformance suite on MemStore; the connect paths
in `migrations.rs`/`connect.rs` against a real Postgres 16.

## Risks

| # | Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|---|
| R-1 | A chat continuing a promoted step meets a lease taken by accept | Low | Medium | Accept refuses while a chat is live (`engine.rs:1366`, `command.rs:1136`); `an_unleased_fence_is_refused_on_a_leased_run` pins the refusal if it ever happens. Also: a park whose best-effort `release_lease` failed (`engine.rs:1877-1890`, warned) leaves `lease_owner` set, and a chat on that promoted step is `Fenced` until the owner's sweep gives the lease back (D140); loud, never silent (blueprint F-11) |
| R-2 | T8 rewrites many lease tests and a meaning drifts | Medium | High | Each rewritten case keeps its name and asserts the same outcome; the reviewer diffs old and new assertions per case |
| R-3 | `ttl = 0` expiry on Postgres is racy against `clock_timestamp()` resolution | Low | Low | `<=` comparisons (`write.rs:3733`, `:3800`); a zero-TTL lease is expired at the next statement |
| R-4 | `.sqlx` churn across four tasks | Certain | Low | `cargo sqlx prepare --check` at every task's gate |
| R-5 | ONNX Runtime / keyring / Postgres env in the cloud box | Certain here | Low | Env from team memory `mod-34-first-pass-in-tree`; recorded in the PR test plan |

## Validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-features --all-targets -- -D warnings`
- `USERNAME=htui-ci RUST_BACKTRACE=0 HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features
  --no-fail-fast -- --test-threads=2` (pre-existing failure on main:
  `every_provider_failure_leaves_a_valid_prompt`)
- `cargo sqlx prepare --check`
- `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`
- Pins: migrations unchanged (applied `1..=7`, `TABLES` 39, commented columns 34); conformance case
  counts move by the cases added, named.

## Acceptance

- A process whose run was adopted cannot append, re-usage or settle that run's step; chats still
  record.
- A fresh transcript batch that collides fails the turn loudly.
- An older quota never overwrites a newer one; an agent edit from a stale view diverges.
- `last_seen_at` advances while the TUI is online.
- A headless connect never migrates and refuses pending, newer or dirty schema and a build below the
  target; a TUI below the target says so and runs.
- Lease expiry and every lease comparison use Postgres's clock.

## Where the PRD, ANA or tree disagree

- **ANA-16 §6.1** cites stale lines (`write.rs:657-685`, `:696-705`, `:3022-3050`, `:832`,
  `pg/mod.rs:449-452`, `store_worker.rs:1128-1135`). Current: `write.rs:946-973`, `:985-1011`,
  `:4072-4102`, `:1113-1136`, `pg/mod.rs:602-606`, `store_worker.rs:1449-1494`. Left as written.
- **ANA-16 C7** says "No `UPDATE box` in `write.rs`". MOD-7 since added `edit_box` (CAS) and
  `record_box_probe` (machine facts); C7 holds, so D6 only pins it.
- **PRD D2** "returns what it wrote": refined by OQ-2.
- **Trait docs** name an offline pending-buffer upload that no longer exists (D3 fixes the docs).

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Five `WriteStore` implementors, no others | true | grep `impl.*WriteStore for`: `pg/write.rs:666`, `writer.rs:351`, `mem.rs:5559`, `htui-agent/src/conformance.rs:711`, `htui-agent/tests/recorder.rs:391` |
| The three step writes filter on step id only | true | `write.rs:946-973`, `:985-1011`, `:4072-4102` |
| MemStore keeps lease owners in a side map | true | `mem.rs:181` `lease_owners: HashMap<RunId, Uuid>` |
| `lease_owner` is cleared only by `release_lease` | true | grep `lease_owner` in `write.rs`: set at `:3634`, `:3740`, `:3794`; NULL only at `:3845` |
| Chat runs have `lease_owner` NULL | true | `start_chat_run` `write.rs:1408-1441` sets no lease column |
| A park releases the lease | true | case `a_parked_run_releases_its_lease_and_an_answer_takes_it`, `htui-orch/src/conformance.rs:3964` |
| Accept refuses while a chat is live | true | `engine.rs:1366` passes `chat_live` to `accept_enabled`, which refuses at `command.rs:1136` |
| The engine builds its recorder with no owner | true | `engine.rs:5271-5277` |
| Recorder has opt-in builders | true | `with_run_cap` at `engine.rs:5281`; `with_quota_latch` at `agent_worker.rs:3114` |
| `flush` re-offers `unflushed` in the same call as fresh rows | true | `record.rs:1024-1050` |
| `RecordError` maps to `DriverError` by an exhaustive match | true | `record.rs:261-269` (D3 adds no variant) |
| No non-test exhaustive `match` on `StoreError` | true | grep `StoreError::… =>`: only single-arm matches with a wildcard (`connection.rs:205-208`, `hierarchy.rs:638`, `store_worker.rs:1973`, `record.rs:1183`) |
| Step prompt and tree writes precede `finish_step` | true | `engine.rs:3106` < `:3144` < `:3189`; `:3707` < `:3726` < `:3790` |
| `interrupt_step` is called by the sweep | true | `engine.rs:9280` `FailedBy::Sweep` |
| `Agent` has `updated_at` | true | `htui-core/src/model/agent.rs:59` |
| `upsert_agent` has no production caller; 65 hits | true | grep `upsert_agent(` workspace-wide = 65; non-test hits are the trait, impls and pass-throughs |
| `CasOutcome` and `cas_miss` exist | true | `traits.rs:1974-1979`; `write.rs:81` |
| `edit_box` is a CAS on `edit_version` | true | `write.rs:1350-1354` |
| Quota write has no ordering guard; zero rows is `NotFound` | true | `write.rs:1113-1136` |
| `last_seen_at` written only at registration | true | `pg/mod.rs:456-471` and the insert default |
| Unknown `app_setting` keys are ignored | true | keyed lookups at `connect.rs:616-637`, `prompt/settings.rs:647`, `:696`; `app_settings()` returns a map (`pg/read.rs:1568-1574`) |
| `semver` is a workspace dependency | true | `Cargo.toml:83` |
| `connect_with` never migrates by itself | true | `pg/mod.rs:168-193`; `apply_migrations` `:234-237` is the only `MIGRATOR.run` |
| `concepts.rs` is the one non-interactive connect | true | `concepts.rs:52-62`; other callers are the TUI dial and tests |
| Lease methods take caller instants | true | `traits.rs:959-1037` |
| MemStore has no clock | true | wrappers call `Utc::now()` (`mem.rs:5926-5973`); lease inner fns use caller instants |
| `TestClock` lives in `htui-orch` | true | `htui-orch/src/fake.rs:791-841` |
| Next migration would be 0008; none needed | true | `ls htui-store/migrations` ends at `0007_skill_attachments.sql`; D1-D11 add no DDL |
| T1-T8 are pairwise serial | true | every adjacent pair shares `traits.rs`/`mem.rs`/`pg/write.rs` or `pg/mod.rs` (Tasks table) |
