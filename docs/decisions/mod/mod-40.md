# MOD-40 - Multi-writer store hardening (done, 2026-09-29)

**Requirements:** `R-ID-3`, `R-HIS-1`, `R-STO-5` (amended 2026-09-26 by maintainer decision: a
headless process never migrates; it refuses and reports).
**Origin:** ANA-16 (`docs/ANA-16.md` §6.1 gaps C1-C8, §8 item 1). It unblocks MOD-41.
**Artifacts:** PRD
[`.claude/prds/mod-40-multi-writer-hardening.prd.md`](../../../.claude/prds/mod-40-multi-writer-hardening.prd.md),
plan
[`.claude/plans/mod-40-multi-writer-hardening.plan.md`](../../../.claude/plans/mod-40-multi-writer-hardening.plan.md)
and blueprints `.claude/plans/mod-40-multi-writer-hardening.blueprint.md` (milestone 1),
`.claude/plans/mod-40-m2-m3.blueprint.md` and `.claude/plans/mod-40-m4.blueprint.md`. Routed as a PRD (C3 and C4 fired).
**Decisions:** maintainer, 2026-09-26: R-STO-5 amended ("amend"), PRD gate D1-D3 as recommended
("all recomm"), plan confirmed with OQ-1 and OQ-2 as recommended.
**Commits:** `e54368b`..`9920756` on `claude/project-thread-bqiyzj`, four milestones and one review
round. No migration: the next one is still `0008`.

## What shipped

**Milestone 1, step fence (C1, C8).** `append_events`, `set_step_usage` and `finish_step` take a
`StepFence` (`traits.rs`): `Lease(owner)` for the engine, `Unleased` for chats. Postgres checks it in
the same statement, through a CTE that takes `FOR SHARE OF r` on the run row and compares
`lease_owner` with `IS NOT DISTINCT FROM`; a miss writes nothing and answers the new
`StoreError::Fenced { step }`. MemStore mirrors it. The recorder carries the fence
(`Recorder::with_fence`) and flushes re-offered rows as a replay call apart from fresh rows, so a
short fresh insert is a loud `RecordError::Store(Constraint)` and is not re-queued. A process that
wakes from suspend after its run was adopted now writes nothing to its old step.

**Milestone 2, ordered writes (C3, C6, C7).** `set_agent_box_quota` answers `Result<bool>` and only
writes when `quota_at <= $4`, so an older quota is a no-op on both stores. `upsert_agent(&Agent,
expected)` is a compare-and-set on `updated_at` answering `CasOutcome<Agent>` (`None` expects no
row); every caller passes the token, test callers through `fixtures::edit_agent`. C7 was already
met by `edit_box`'s compare-and-set; it is now pinned by a trait-doc invariant and an existing test.

**Milestone 3, box and schema (C4 heartbeat, C5).** `PgStore::touch_box` sets
`box.last_seen_at = clock_timestamp()`; the store worker beats every `BOX_HEARTBEAT` while online,
with the period on `Started`. `PgStore::connect_headless` never migrates and never creates
`_sqlx_migrations` (it probes with `to_regclass`); it refuses with `HeadlessError::MigrationsPending`
or `BelowTarget`. Migrating raises `app_setting` `htui_target_version` to this build's version in one
transaction (semver, never lowered). A TUI below the target connects and shows "htui X is older
than Y, which last migrated this database; upgrade this box" on the status line. `concepts.rs`
(`--index-items`, `--search-items`) connects headless.

**Milestone 4, database time (C2).** Every lease method takes a TTL (`chrono::TimeDelta`, 0 to 365
days), and Postgres stamps `clock_timestamp() + ttl` for every expiry and every expiry comparison.
`claim_run` keeps its `at` argument for `started_at`; `release_lease(run, owner)` takes no time.
`Clock`, `SystemClock` and `TestClock` moved to `htui-core`, and `htui-orch` re-exports them. Each
MemStore handle holds its own clock; the default is untruncated `Utc::now()`. The heartbeat still
fences on local elapsed time (OQ-2), and `LeaseHeld` is decided by box.

**Review round** (`rust-reviewer`, approve with fixes, all seven applied):
- A walk refused with `Fenced` mid-session now releases its isolation guards and ends as
  `LeaseLost`, like an abandoned heartbeat. Before this, it held the repository mutex forever
  (`0f361ea`).
- The box heartbeat's end wakes the store loop (`69eac5e`).
- The heartbeat, race and error-context tests now fail when they should.
- Stale offline-buffer comments were rewritten, and "cannot reach Postgres" is only said when
  Postgres is unreachable.

## Decisions worth keeping

**Only the three named writes are fenced (OQ-1).** `set_step_prompt` and `upsert_step_tree` run
before the session, so a stale holder that wakes mid-session meets a fenced recorder flush first.
`interrupt_step` is the sweep's own write and must stay unfenced. The settle tail
(`record_commits` and the sink's output document) is unfenced too. A holder that wakes after the
step is `done` reaches those writes before its fenced `finish_step`. This is left to MOD-41.

**The heartbeat fences on local elapsed time (OQ-2).** Comparing a database-stamped instant with
the local clock would reintroduce the skew C2 removes, so the lease methods keep their `bool`,
`Claim` and `Vec<Run>` answers. The local fence reads the wall clock, so a wall-clock step (NTP,
suspend) during a lease still moves it (blueprint F-38). A monotonic clock is MOD-41's.

**A headless process never migrates (R-STO-5 amended).** A TUI with someone to confirm still
migrates; a worker refuses and reports. The target version lets an older box know a newer build
migrated the database.

## Left open

- MOD-41 carries the unfenced writes above and the monotonic self-fence.
- The pre-existing MemStore project-delete gap (`item_link.proposed_by_step_id`) is untouched.

## Pins after this item

The store conformance suite has 96 `CASES` (was 83) and 14 `READ_CASES`. `htui-orch` has 73 `CASES`
(was 72). `crates/htui-store/.sqlx` has 288 files (was 281). `StoreRequest`, `StoreReply`,
`MIRRORED_TABLES` and the snapshots are unchanged. The full workspace suite, after merging main at `2bc84af`, passed
2504 and failed 0, with 26 ignored.
