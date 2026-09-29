# Blueprint: MOD-40, milestone 4 (T7, T8)

**Status**: proposed (2026-09-29). Milestone 4. Continues
`.claude/plans/mod-40-multi-writer-hardening.blueprint.md` (milestone 1, F-1…F-12, B1…B9) and
`.claude/plans/mod-40-m2-m3.blueprint.md` (milestones 2 and 3, F-13…F-29, B10…B20): findings
continue at **F-30**, decisions at **B21**. A **Blocker** means the plan, read literally, fails an
existing test, does not compile at a task boundary, or cannot pin its own claim. The Fix column is
what the implementer builds.

**Plan**: `.claude/plans/mod-40-multi-writer-hardening.plan.md`, CONFIRMED 2026-09-26, OQ-1 and
OQ-2 as recommended (OQ-2: the lease methods take a TTL, Postgres stamps `clock_timestamp()`, the
heartbeat self-fences on **local** elapsed time with `written` = local send time + TTL, nothing
returns the stamp). Its D10, D11, T7 and T8 are the scope here, amended only where §0 says so.
**PRD**: `.claude/prds/mod-40-multi-writer-hardening.prd.md`; PRD D2 wins over this blueprint,
refined by OQ-2 as the plan says.

**Verified at**: `cd49a3b` (milestone 1 complete, milestones 2-3 blueprint committed), read from
`git archive cd49a3b` into the scratchpad because milestone 2 is being implemented in the working
tree. Paths are relative to `crates/`. **Line numbers are at `cd49a3b`** unless a line says
`@main`. Milestones 2 and 3 move `traits.rs`, `mem.rs`, `pg/write.rs`, `writer.rs`,
`pg_criteria.rs` and the two conformance count pins before T7 starts, and the branch merges
`origin/main` (`ca2fb1f`, §11) before the PR, so the implementer re-greps the quoted anchor text,
never the number alone. Every fact was read with `grep`/`sed`; `graphify-out/` does not exist and
Gortex is not reachable in this session.

**Layout**: §0 findings and decisions; §1 build order; §2 T7; §3 T8 production code; §4 T8 new
tests; §5 T8 test rewrites, file by file, case by case; §6 parallel groups; §7 `.sqlx`; §8
file-by-file checklist; §9 gates; §10 PR #13 (MOD-58); §11 main.

**Probes** (Postgres 16.13, `localhost:5432`, scratch database `mod40_m4c` built from migrations
`0001`–`0007` with `psql`, dropped afterwards; a scratch probe test in the scratchpad copy of
`cd49a3b`, never in the working tree):
- **P-11** (MemStore stamps). 20 000 consecutive `MemStore::set_setting` compare-and-sets on one
  row, debug build, each token taken from the previous answer: **9 367** pairs of consecutive
  writes share one microsecond. Today's stamps are `Utc::now()` in nanoseconds, so none collide.
- **P-12** (one write, two stamps). `UPDATE run SET lease_expires_at = clock_timestamp() + $n *
  interval '1 microsecond' … RETURNING updated_at - (lease_expires_at - ttl)`: `46 µs` for a
  single-row refresh; `838 µs` for the claim's statement inside its transaction. The SET's
  `clock_timestamp()` is evaluated before the `BEFORE UPDATE` trigger's
  (`0001_init.sql:21-24`, `NEW.updated_at := clock_timestamp()`), so on every lease write
  `updated_at - (lease_expires_at - ttl)` is `≥ 0` and a few hundred µs. `release_lease`'s
  `updated_at - lease_expires_at` = `33 µs`. A lease written with `ttl = 0` reads
  `lease_expires_at <= clock_timestamp()` = `t` at the next statement.
- **P-13** (interval arithmetic). Untyped `PREPARE` (what sqlx's describe does) of the §3.2
  statements infers `{uuid,uuid,bigint}` (refresh), `{uuid,uuid,uuid,bigint}` (take), `{uuid,uuid}`
  (release), `{uuid,uuid,uuid,timestamptz,bigint}` (claim). `86400000000::bigint * interval '1
  microsecond'` is `24:00:00` with a **zero** day field; under `SET TimeZone = 'Europe/Rome'`,
  `timestamptz '2026-10-24 12:00+00'` plus it lands at `2026-10-25 12:00 UTC`, while `+ interval
  '1 day'` lands at `13:00 UTC` (the DST night is 25 h). `31536000000000` µs (one year) is exact;
  `9007199254740993` µs (> 2^53) comes back `…992` (the product goes through `float8`).
- **P-14** (the §3.2 statements on rows). A stranger's take of a lease one day live → `UPDATE 0`;
  the same take after `lease_expires_at` is planted one second in the past → `UPDATE 1`,
  `lease_expires_at - updated_at` = `00:00:59.999961` for a 60 s TTL. The adopt CTE over three
  lapsed runs and one live one adopts the three, each `lease_expires_at - updated_at` =
  `00:04:59.9999xx` for a 5 min TTL, and skips the live one.

**Scope**: no migration (0008 stays free). New public items: `htui_core::clock::{Clock,
SystemClock}`, `htui_core::clock::{TestClock, epoch}` (feature `test-support`),
`htui_core::store::{MAX_LEASE_TTL, lease_ttl_micros}`, `MemStore::with_clock`,
`MemStore::handle_at` (feature `test-support`). `htui_orch::{Clock, SystemClock}`,
`htui_orch::isolate::{Clock, SystemClock}` and `htui_orch::fake::TestClock` keep their paths as
re-exports. Five `WriteStore` signatures change across five implementors. Store `CASES` **95 →
96** (T8 adds `a_lease_ttl_out_of_range_is_refused`; 95 is milestone 3's end state per the m2-m3
blueprint). Orch `CASES` unchanged (73). `.sqlx/` unchanged in count (§7: 5 replaced).

**House style (carried)**: `unsafe_code = "forbid"`; `missing_docs`,
`missing_debug_implementations`, `unused_qualifications` warn and clippy runs `-D warnings`;
rustdoc denies broken/private intra-doc links, so an ungated item's doc never links a
`test-support` item (the note `isolate.rs:250-252` already carries). `max_width = 100`. Every
commit compiles; red first, then green. `every_cross_referenced_test_name_exists`
(`htui-core/src/store/conformance.rs:11960`) scans **`conformance.rs`'s own source**: a backticked
snake_case name with ≥ 4 underscores there must be a fn in `conformance.rs` or `mem.rs`, and a
test in another file is spelled `pg_criteria.rs::name`.

---
## 0. Findings (continued from the m2-m3 blueprint's F-13 … F-29)

| # | Severity | Plan says | Tree / probe | Fix |
|---|---|---|---|---|
| **F-30** | **Blocker** (five existing cases would flake) | D11: `MemStore::with_clock`, "the default is `SystemClock`, and every `Utc::now()` in the wrappers reads it"; T7: "no behaviour change". | `SystemClock` truncates to µs (`isolate.rs:265-270`, and that is `Clock`'s contract, `:254-261`). Today's 61 wrapper stamps are untruncated `Utc::now()` (`mem.rs:5589-6237`). P-11: 47 % of back-to-back MemStore compare-and-sets share one µs. Five store cases assert that one edit moved the `updated_at` token of a row a previous edit in the same case stamped: `conformance.rs:2140`, `:2417`, `:2909`, `:6747` and `:6960` (`skill_binding_attach_change_detach_are_compare_and_set`, "the change moved the token"). Under a µs default each fails whenever its two writes share a µs, and a spent token then reads current (ABA). | **B21**: a `MemStore` with no clock injected keeps today's stamps exactly: `Utc::now()`, untruncated. `with_clock` opts a handle in; the wrappers read `self.now()`, which is the injected clock or that wall read. No `Clock` impl is untruncated, so the trait's contract holds. D11's T7 test (`a_mem_store_reads_its_clock`) is unchanged. |
| **F-31** | Major (does not compile) | D11: `TestClock` moves from `htui-orch/src/fake.rs:791-841` into `htui-core`. | `TestClock::default()` is `Self::at(epoch())` with `epoch` from `htui_agent::conformance` (`fake.rs:21`, `:795-799`); `htui-core` cannot depend on `htui-agent` (`htui-agent/Cargo.toml:18` depends the other way). | **B22**: `epoch()` moves to `htui_core::clock` beside `TestClock` (feature `test-support`); `htui_agent::conformance::epoch()` (`htui-agent/src/conformance.rs:282-284`) delegates to it, keeping its path, doc and value (`2026-09-06T00:00:00Z`). `htui-agent`'s `test-support` already enables `htui-core/test-support` (`htui-agent/Cargo.toml:15`). |
| **F-32** | **Blocker** (every restart case) | D11: "`MemStore::with_clock` … so tests share one `TestClock`". | The orch suite models a second process with `FakeOrchestrator::restarted` (`fake.rs:1343-1375`): the **same** rows (`store.clone()`), a **new** clock `RESTART_GAP` (10 min) later, a new owner. The sweep after a restart adopts because the second process passed its own, later `now` (`engine.rs:1961-1967`). With lease expiry judged by one store clock, a store clock shared with the first process says the lease is live and every adoption case (orch conformance `:4107`, `:4288`, `:4354`, … 16 `restarted()` sites; `engine.rs:8995-9230`) adopts nothing. `gix_isolator.rs:2060-2065` builds its own second process the same way and hands its engines `&fix.orch.store` (`:88`). | **B23**: a clock belongs to a **handle**. `MemStore::clone` shares rows and faults and copies the clock; `with_clock` replaces it on that handle only. `TestClock` becomes a shared handle (`Clone` shares the instant). `FakeOrchestrator::demo` clocks its store on its own `TestClock`; `restarted` clocks the cloned store on the new one. `gix_isolator`'s `second_process` returns a clocked store and `engine_as!` takes the store it builds on. Each process then judges a lease exactly as it did when it passed its own `now`. |
| **F-33** | Major (cases that stage a stranger at a later instant) | T8: "conformance cases that expired a lease by passing a future `now` rewritten to expire it by the store's clock (MemStore: `TestClock::advance`)". | In the engine's lease tests the stranger's later instant and the walk's own clock are **different** clocks: `a_walk_whose_lease_is_taken_is_abandoned_and_writes_nothing` (`engine.rs:8182-8283`) adopts at `now + ttl + 1 s` while the walk's heartbeat reads the harness clock; advancing that clock by `ttl + 1 s` would fence the heartbeat itself (`Expired`), not abandon it (`Abandoned`), and the case would test the other branch. The same shape: `swept_while_a_stranger_takes_over` (`:9025-9095`, take at `later`). | **B24**: `MemStore::handle_at(now)` (feature `test-support`): a clone whose clock is a `TestClock` frozen at `now`, i.e. the stranger's view. `advance` is used only where the old case moved the one clock every party reads. |
| **F-34** | Major (maintainer: amends D10's `started_at` clause) | D10: "Postgres stamps `clock_timestamp()` for `started_at`, `lease_expires_at` and every expiry comparison"; D10's signature `claim_run(run, box, owner, ttl)` drops `at`. | `at` writes `started_at` and, on `MissingTags`, `finished_at` (`pg/write.rs:3602`, `:3695`; `mem.rs:3926`, `:3993`). Both are pinned as the **caller's** clock: `conformance.rs:4428` ("`at` is the caller's clock"), `:4863`, `mem.rs:8171`, and the rule is stated for the sibling column at `conformance.rs:10722` ("`finished_at` is the caller's clock, not the store's"). Every other run and step stamp of the walk is the engine's `Clock` (`queued_at`, `finish_run`, step `started_at`/`finished_at`, notes, documents; MOD-4 plan D8, `isolate.rs:246-252`). No cross-process comparison reads `run.started_at`: its one production reader is the Runs pane's display (`htui/src/ui/tabs/backlog/detail/runs.rs:893`), and `htui/tests/backlog.rs:278-285` stamps it with a `Fixed` clock so the Runs snapshots (`runs_reject_note`, `runs_artifact`, `runs_closeout_*`) are byte-stable. A store-stamped `started_at` beside caller-stamped `finished_at` orders a skewed worker's run as finishing before it started. | **B25**: `at` stays a caller instant and keeps writing `started_at` and the `MissingTags` `finished_at`; only `lease_until` becomes `ttl`. C2 is about **lease** comparisons between processes, and `started_at` is in none. Signature: `claim_run(run, box_id, owner, at, ttl)`. The PRD's success-metric row "every lease expiry and `started_at` written by the lease methods" is corrected at bookkeeping. |
| **F-35** | Major (reintroduces skew) | D10 changes the writes; the engine's reads are not named. | `Engine::take_lease` decides `LeaseHeld` by `row.lease_expires_at.is_some_and(\|until\| until > self.now())` (`engine.rs:1781`): a store-stamped expiry against the local clock. A worker a day ahead reads a live foreign lease as lapsed and says `RunStatus { expected: "…, executing on this box" }`. It is the only production comparison of `lease_expires_at` (grep of `htui*/src`). | **B26**: no clock is read. After a zero-row take on a `running`/`awaiting_approval` run, `executing_box_id == this box` means the store refused on the one predicate left, a live foreign lease, so `LeaseHeld`; otherwise the box rule, `RunStatus`. One wording moves: a run on **another** box under a **live** lease was `LeaseHeld` and is `RunStatus` now (this box could never take it). No test pins that pair (`engine.rs:10033-10088` pins the lapsed one, which is `RunStatus` both ways). New pin: §4.3. |
| **F-36** | Minor (a choice the plan leaves open) | D10: "the lease methods take a TTL"; type unnamed. | `LeaseTimes::ttl` is a `chrono::TimeDelta` (`recover.rs:44-47`) and every other time in `WriteStore` is chrono. `TimeDelta` can be negative and can overflow `DateTime + TimeDelta` (a panic) and `num_microseconds()` (`None`). A `$n::interval` bind must be `PgInterval` exactly (sqlx's macro type check is `same_type`, `sqlx-macros-core-0.9.0/src/query/args.rs:69-78`; `PgInterval` is the only interval mapping, `sqlx-postgres-0.9.0/src/type_checking.rs:25`), reached through a fallible `TryFrom`. P-13: `interval '1 day'` is DST-dependent in a non-UTC session; a µs count times `interval '1 microsecond'` is absolute and exact below 2^53. | **B27**: `TimeDelta`. One shared gate, `lease_ttl_micros(ttl) -> Result<i64>`: `0 ≤ ttl ≤ MAX_LEASE_TTL` (365 days, the bound `LeaseTimes` already enforces, `recover.rs:37-38`), else `Constraint`, checked before any row is read; its answer is the µs count Postgres binds (`$n::bigint * interval '1 microsecond'`) and MemStore adds (`TimeDelta::microseconds(n)`), so both stores add the same truncated span. One conformance case (§4.1). `recover.rs`'s private bound becomes `MAX_LEASE_TTL.num_seconds()` (const). |
| **F-37** | Minor | OQ-2: "the heartbeat self-fences on local elapsed time with `written` = local send time + ttl". | `heartbeat` already computes `until = clock.now() + ttl` **before** it calls `refresh` and fences on `until - margin` (`recover.rs:136-150`); only the closure's argument (`until`) is a store input. The engine builds every `written` from `self.now()` taken before the store call (`engine.rs:686-695`, `:1802-1808`). | **B28**: the closure takes the TTL: `F: FnMut(TimeDelta) -> Fut`, called with `times.ttl`; nothing else in `heartbeat` changes. The engine passes `now + ttl` as `written`, where `now` is read before the store call. One test assertion changes (§5.8). |
| **F-38** | Minor (record; MOD-41) | OQ-2: "a duration measured on one clock is skew-free". | True for an **offset** between hosts. The Postgres stamp is taken after the local send instant in real time (the statement runs after it is sent), so the stored expiry is at or after the local fence plus the margin, up to the two clocks' rate drift over one TTL. But the fence reads `Clock`, which is the **wall** clock (`SystemClock`), so a wall-clock step during a lease (NTP, suspend/resume) still moves it. A monotonic `tokio::time::Instant` would be step-proof and is already what the heartbeat sleeps on. | No change in MOD-40 (C2 is the offset). Named in the MOD-41 note and in `heartbeat`'s doc (§3.5). |
| **F-39** | Major (the rewritten cases cannot pin their value on Postgres) | T8: conformance cases re-expressed "meaning unchanged". | A generic case holds `&S: WriteStore` and cannot read the store's clock; nine assertions pin an exact expiry (`conformance.rs:4437`, `:5148`, `:5161`, `:5202`, `:5291`, `:5303`, `:5348`, `:5447`, `:5462`). Postgres cannot fast-forward. | **B29**: `assert_leased_for(case, &run, ttl, what)` reads the lease against the row's **own** `updated_at`, stamped by the same write on the same clock: `ttl - 1 s < lease_expires_at - updated_at <= ttl` (MemStore: `== ttl`; Postgres: `ttl` less the P-12 gap). A released lease is `assert_leased_for(…, TimeDelta::zero(), …)`. Expired = written with `ttl = 0` (P-12); live = minutes or days. A refused write is pinned as "the whole row is unchanged", stronger than the old one-column check. |
| **F-40** | Minor (test churn) | T8 lists the conformance lines at `edf19c0`. | Re-grepped at `cd49a3b` and `@main`: 67 lease calls in `htui-core/src/store/conformance.rs`, 19 in `mem.rs`'s tests (+4 `@main`, PR #13), 11 in `pg_criteria.rs` (+4 `@main`, PR #13), 5 in `htui-orch/src/conformance.rs`, 26 in `engine.rs`'s tests, 2 in `recover.rs`, 1 each in `review_loop.rs`, `run_worker.rs`, 2 in `recorder.rs`'s tests. `htui-orch/tests/gix_isolator.rs` has none but needs B23. `htui/tests/*` has none. | §5, case by case; §6 splits them into five disjoint groups. |
| **F-41** | Minor | T8 names `run_worker.rs:3572`. | `run_worker`'s fixture store is `MemStore` with no clock (`run_worker.rs:2721-2731`) and its runtimes use `TokioClock` (`:2674-2692`, `:2750`). Lease expiry there is judged on wall time; the tests that need a lapse get it from a release (`:3024`, `:4043`, the outage case `:4389` via the dead-walk pre-pass) or from `stranded` (`:3461-3500`), which claims with an hour-old lease. `:3572` compares an adopted expiry with `Utc::now()`: the store's clock **is** the wall clock there. | The fixture store stays unclocked (B21); `TokioClock` and `recover.rs`'s `PausedClock` stay private test types (they implement the re-exported trait; nothing moves). `stranded` claims with `ttl = 0`; `:3572` is unchanged. |
| **F-42** | Minor (record) | — | A frozen `TestClock` stamps every write of a clocked handle with one instant, so a compare-and-set on `updated_at` cannot tell two edits at that instant apart. The orch suite's CAS edits (`htui-orch/src/conformance.rs:990`, `engine.rs:7241`, `:10435`, `:10565`, `:12997`) each read a fresh token first, which is always consistent, and no orch case asserts a token moved. | `with_clock`'s doc says so: advance between two edits whose staleness a case checks. |

### 0a. Decisions (this blueprint's; the plan's D-numbers are unchanged)

- **B21** (F-30): `MemStore` keeps untruncated `Utc::now()` unless a clock is injected.
- **B22** (F-31): `epoch()` moves to `htui_core::clock`; `htui_agent::conformance::epoch` delegates.
- **B23** (F-32): the clock is per handle; `TestClock` clones share their instant; `FakeOrchestrator`
  and `gix_isolator`'s second process clock their own store handle.
- **B24** (F-33): `MemStore::handle_at(now)` stands in for a stranger's clock.
- **B25** (F-34): `claim_run` keeps `at` for `started_at` and the `MissingTags` `finished_at`.
- **B26** (F-35): `LeaseHeld` is decided by the box, not by a clock.
- **B27** (F-36): the TTL is a `TimeDelta` gated by `lease_ttl_micros` (0 ≤ ttl ≤ 365 d), bound as a
  µs `bigint` times `interval '1 microsecond'`.
- **B28** (F-37): `heartbeat`'s refresh closure takes the TTL; `written` = local send instant + TTL.
- **B29** (F-39): rewritten cases pin a lease against the row's own `updated_at`; expiry is `ttl = 0`.
- **B30**: the one new store case, `a_lease_ttl_out_of_range_is_refused`, is appended to `CASES` after
  milestone 2's last (`"upsert_agent_with_the_current_token_applies"`, B20); its arm goes before
  `other => panic!`, its body after that case's body.

---
## Milestone 4 — database time (T7, T8)

### 1. Build order

| Task | Crates | Commits (each compiles) | Gate |
|---|---|---|---|
| T7 | htui-core, htui-agent (one fn body), htui-orch (re-exports, fake, one test file) | (1) red: `htui-core/src/clock.rs` with the three moved types and `epoch` (§2.1), the re-exports (§2.3), `MemStore::{with_clock, handle_at}` and the private `MemClock` **not yet read by any wrapper** (§2.4), `FakeOrchestrator` and `gix_isolator` clocking their handles (§2.5), and the T7 tests (§2.6): `a_mem_store_reads_its_clock` fails (the wrappers still call `Utc::now()`); (2) green: the 61 wrapper stamps and the two inherent test writers read `self.now()`. | §9 T7 |
| T8 | all five | (1) red: the five signatures on the trait, `lease_ttl_micros`/`MAX_LEASE_TTL` (§3.1), every implementor forwarding the TTL but **computing the old instants from it** (Pg: the old statements and `.sqlx` files, binding caller instants derived from the TTL: `at + ttl` for the claim, `Utc::now() + ttl` / `Utc::now()` for the others; Mem: `now + ttl` from its handle clock, which is already the store's clock, so MemStore is green except the new gate), the engine and heartbeat (§3.5), every test call site rewritten (§5), the new tests (§4); red: `a_lease_ttl_out_of_range_is_refused` (no gate yet), `pg_criteria.rs::lease_times_are_the_databases` (caller time), `a_skewed_engine_still_names_a_live_lease_held` (the old clock check); (2) green: the gate on both stores, the §3.2 SQL, `.sqlx`, B26, docs. | §9 T8 |

T7 and T8 are serial (both edit `mem.rs`; T8's MemStore half needs T7's handle clock). Inside
T8 commit (1), after G0 (§6) lands the signatures, the test rewrites run as four parallel groups
over disjoint files.

### 2. T7 — `Clock` into `htui-core` (D11, B21-B24)

#### 2.1 `htui-core/src/clock.rs` (new)

`htui-core/src/lib.rs`: add `pub mod clock;` between `pub mod model;`'s neighbours in alphabetical
order (before `pub mod model;`). Ungated: `MemStore` is ungated and holds a clock.

```rust
//! The one place an instant enters the walk (MOD-4 plan D8) and, since MOD-40 plan D11, a
//! [`MemStore`](crate::store::MemStore) handle.
//!
//! [`Clock`] and [`SystemClock`] moved here from `htui-orch`'s `isolate.rs`, and `TestClock` and
//! `epoch` from its `fake.rs` (feature `test-support`), so a store and the engine writing through
//! it can read one clock. `htui-orch` re-exports all three at their old paths.

#[cfg(feature = "test-support")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "test-support")]
use chrono::TimeDelta;
use chrono::{DateTime, SubsecRound as _, Utc};

use crate::model::TIMESTAMPTZ_DIGITS;

/// The one place an instant enters the walk (MOD-4 plan D8).
///
/// Every seam writer takes its `at` from the caller, so the engine owns the clock; and
/// `docs/ANA-2.md:1766-1768` requires the harness's settle snapshots to be sleep-free, so the
/// engine must be able to be handed a clock a test moves. `TestClock` is that clock (behind
/// `test-support`, so it is not linked from here: a doc link into a gated item is
/// `broken_intra_doc_links` in a plain `cargo doc`).
///
/// Since MOD-40 plan D10 a lease's expiry is the **store's** clock plus a TTL, never an instant
/// read here: a clock is only compared with another reading of itself.
pub trait Clock: Send + Sync {
    /// Now, already truncated to the column's resolution.
    ///
    /// The truncation is the contract, not an implementation detail: `TIMESTAMPTZ` keeps
    /// microseconds and `chrono` keeps nanoseconds, so an untruncated instant round-trips
    /// differently through Postgres than through `MemStore` and the two backends disagree about a
    /// column neither changed (`crates/htui-core/src/model/run.rs:272`).
    fn now(&self) -> DateTime<Utc>;
}

/// The production [`Clock`]: `Utc::now()`, truncated.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now().trunc_subsecs(TIMESTAMPTZ_DIGITS)
    }
}

/// The suite's fixed instant, `2026-09-06T00:00:00Z`: where every `TestClock` starts, and the
/// instant `htui_agent::conformance::epoch()` answers (it delegates here, MOD-40 blueprint B22).
///
/// # Panics
///
/// Never: the constant is a valid instant.
#[cfg(feature = "test-support")]
#[must_use]
pub fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(1_788_393_600_000).expect("the suite epoch is a valid instant")
}

/// A [`Clock`] a test moves, starting at [`epoch`].
///
/// [`epoch`] is the instant every `FakeDriver` envelope is stamped from, so starting here means a
/// step's store rows and its session rows share one origin and a snapshot of both reads as one
/// timeline. [`advance`] is how a case elapses time; no case ever sleeps.
///
/// **A clone is the same clock** (MOD-40 blueprint B23): it reads and moves one shared instant,
/// which is how a harness hands its engine and its `MemStore` handle one clock. A second,
/// independent clock is [`TestClock::at`].
///
/// [`advance`]: TestClock::advance
#[cfg(feature = "test-support")]
#[derive(Debug, Clone)]
pub struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

#[cfg(feature = "test-support")]
impl Default for TestClock {
    fn default() -> Self {
        Self::at(epoch())
    }
}

#[cfg(feature = "test-support")]
impl TestClock {
    /// A clock at [`epoch`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A new, independent clock at `now`, truncated like every other instant the walk hands a
    /// writer (plan D8).
    #[must_use]
    pub fn at(now: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(now.trunc_subsecs(TIMESTAMPTZ_DIGITS))),
        }
    }

    /// Move the clock, and every clone of it, forward (or back, for a test that wants to).
    pub fn advance(&self, by: TimeDelta) {
        let mut now = self
            .now
            .lock()
            .expect("no panic holds the test clock's lock");
        *now = (*now + by).trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }

    /// Put the clock, and every clone of it, at an exact instant.
    pub fn set(&self, to: DateTime<Utc>) {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock") = to.trunc_subsecs(TIMESTAMPTZ_DIGITS);
    }
}

#[cfg(feature = "test-support")]
impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        *self
            .now
            .lock()
            .expect("no panic holds the test clock's lock")
    }
}
```

`mod tests` at the end of `clock.rs`: `system_clock_is_microsecond_truncated` moved verbatim from
`isolate.rs:272-290` (`#[cfg(test)]`); `test_clock_starts_at_the_drivers_epoch_and_only_a_test_moves_it`
moved verbatim from `fake.rs:2619-2637` under `#[cfg(all(test, feature = "test-support"))]`, with
`epoch` now `super::epoch`; and one new case beside it:

```rust
    /// MOD-40 blueprint B23: a clone is a second handle on one instant, `at` is a new clock.
    #[test]
    fn a_test_clock_clone_moves_with_it_and_at_does_not() {
        let clock = TestClock::new();
        let handle = clock.clone();
        let other = TestClock::at(clock.now());
        clock.advance(TimeDelta::seconds(5));
        assert_eq!(handle.now(), clock.now(), "a clone reads the instant its original moved");
        assert_eq!(other.now(), epoch(), "`at` started a clock of its own");
        handle.set(epoch());
        assert_eq!(clock.now(), epoch(), "and moving the clone moves the original");
    }
```

#### 2.2 `htui-agent/src/conformance.rs:282-284` (B22)

Body only; the doc (`:270-281`) and the signature stay:

```rust
pub fn epoch() -> DateTime<Utc> {
    htui_core::clock::epoch()
}
```

The `# Panics` paragraph now reads "Never: `htui_core::clock::epoch` is a valid instant."

#### 2.3 `htui-orch`: re-exports at the old paths

- `htui-orch/src/isolate.rs:246-270`: the `Clock` doc, trait, `SystemClock` and its impl are
  replaced by `pub use htui_core::clock::{Clock, SystemClock};`. `:272-290` (`mod tests`) is
  deleted (moved, §2.1). The module doc's `:9-13` paragraph ("`Clock` is here rather than in
  `engine.rs` for a build-order reason …") is replaced by "`Clock` and `SystemClock` live in
  `htui_core::clock` since MOD-40 plan D11 and are re-exported here, so every path that named
  them still does." Drop the imports the compiler then reports unused (`SubsecRound`,
  `TIMESTAMPTZ_DIGITS`, and `DateTime`/`Utc` if nothing else in the file names them).
- `htui-orch/src/lib.rs:60-63`: unchanged (`pub use isolate::{Clock, …, SystemClock}` resolves
  through the re-export).
- `htui-orch/src/fake.rs:782-841`: `TestClock`, its doc and impls are replaced by
  `pub use htui_core::clock::TestClock;`. `:2619-2637` (the moved test) is deleted. `:21`'s
  `epoch` import stays (the file uses it at `:757-758`, `:2500`); drop what the compiler reports
  unused (`Mutex` is still used by the fakes; `Clock` may not be).
- `PausedClock` (`recover.rs:490-514`), `TokioClock` (`htui/src/run_worker.rs:2671-2692`) and
  `Fixed` (`htui/tests/backlog.rs:278-285`) **stay** where they are: each is a private test type
  that implements the trait through its re-exported path, and none is a store's clock (F-41).

#### 2.4 `htui-core/src/store/mem.rs` — the handle's clock (B21, B23, B24)

New private type, above `pub struct MemStore` (`:63`):

```rust
/// Where one [`MemStore`] handle reads "now" (MOD-40 plan D11, blueprint B21, B23).
///
/// `None` is the wall clock **untruncated**, exactly the stamps every `MemStore` wrote before
/// MOD-40: two back-to-back compare-and-sets share one microsecond about half the time (blueprint
/// P-11), and a truncated default would let a spent `updated_at` token read as current.
#[derive(Clone, Default)]
struct MemClock(Option<Arc<dyn Clock>>);

impl std::fmt::Debug for MemClock {
    /// Hand written: [`Clock`] carries no `Debug` supertrait.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "MemClock(injected)" } else { "MemClock(wall)" })
    }
}

impl MemClock {
    fn now(&self) -> DateTime<Utc> {
        self.0.as_ref().map_or_else(Utc::now, |clock| clock.now())
    }
}
```

`MemStore` (`:63-72`) gains, after `state`:

```rust
    /// This handle's clock (MOD-40 plan D11). **Not** shared through clones the way `state` is:
    /// a clone copies it, and [`MemStore::with_clock`] replaces it on one handle, so two handles
    /// on one set of rows can read two clocks, as two processes on one database do (blueprint
    /// B23).
    clock: MemClock,
```

The derive stays `#[derive(Debug, Clone, Default)]`. `from_demo`'s literal (`:300-304`) gains
`clock: MemClock::default(),`. Import `crate::clock::Clock` (and, under `test-support`,
`crate::clock::TestClock`).

Inherent methods, after `set_fault` (`:760-768`):

```rust
    /// This handle, reading `clock` for every stamp and every lease comparison it makes (MOD-40
    /// plan D10, D11). The rows and the fault switches stay shared with every clone; the clock is
    /// this handle's alone.
    ///
    /// A frozen clock stamps every write with one instant, so a compare-and-set on `updated_at`
    /// cannot tell two edits made at it apart: a case that checks a spent token moves the clock
    /// between the two edits (blueprint F-42).
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = MemClock(Some(clock));
        self
    }

    /// A handle on the same rows whose clock is a `TestClock` frozen at `now`: a second
    /// process's view, for a case that stages a stranger at another instant than its own clock
    /// (MOD-40 blueprint B24).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn handle_at(&self, now: DateTime<Utc>) -> Self {
        self.clone().with_clock(Arc::new(TestClock::at(now)))
    }

    /// This handle's "now": its injected clock, else `Utc::now()` untruncated (blueprint B21).
    fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }
```

Commit (2): every `let now = Utc::now();` and `let stamp = Utc::now();` in `impl WriteStore for
MemStore` (`:5589-6237`, **61** at `cd49a3b`; milestone 2 adds none and keeps the ones it
touches, re-grep) and in the two inherent test writers `set_app_setting` (`:515`) and
`set_project_settings` (`:526`) becomes `self.now()`. The `Utc::now()` reads inside `mod tests`
(`:6239` on) are the tests' own and stay. After commit (2):

```bash
awk '/^impl WriteStore for MemStore/,/^#\[cfg\(all\(test/' crates/htui-core/src/store/mem.rs \
  | grep -c 'Utc::now()'          # → 0
```

#### 2.5 Harnesses clock their own handle (B23)

- `htui-orch/src/fake.rs:1304-1330` (`FakeOrchestrator::demo`):

  ```rust
        let clock = TestClock::new();
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
  ```

  and the literal's `clock: TestClock::new(),` becomes `clock,`. The struct doc (`:1253-1254`)
  gains: "The store handle reads [`clock`](Self::clock) (MOD-40 plan D11), so a lease the walk
  takes expires by the clock the case moves."
- `fake.rs:1343-1375` (`restarted`): `let clock = TestClock::at(self.clock.now() + RESTART_GAP);`
  first; `store: self.store.clone().with_clock(Arc::new(clock.clone())),`; `clock,`. Its doc
  (`:1331-1341`) gains: "The store is a new handle on the same rows that reads the new clock, so
  this process judges every lease by its own clock, as a process with its own database session
  would (MOD-40 blueprint B23)." `RESTART_GAP`'s doc (`:1240-1242`) is unchanged and still true.
- `htui-orch/tests/gix_isolator.rs`:
  - `second_process` (`:2056-2065`) returns `(GixIsolator, Uuid, TestClock, MemStore)`, the store
    `self.orch.store.clone().with_clock(Arc::new(clock.clone()))`; its doc gains "and a store
    handle that reads that clock".
  - `engine_as!` (`:57-110`) gains `$store:expr` after `$fix` and builds `store: $store` (`:88`);
    its doc names it. Every call site passes `&fix.orch.store` for the fixture's own process and
    the fourth tuple item for a second process: the `let (isolator, owner, clock) =
    fix.second_process();` sites `:2182`, `:2263`, `:2368`, `:2475`, `:2618` become `let
    (isolator, owner, clock, store) = …` and their `engine_as!` calls (`:2183`, `:2269`,
    `:2286`, `:2328`, `:2399`, `:2476`, `:2546`, `:2620`) pass `&store`; every other
    `engine_as!` call (grep `engine_as!(`) and `Fixture::dispatch` pass `&fix.orch.store`.
  - Main touched this file at `@main:3333-3368` (a disjoint hunk, §11).

`htui-orch/src/conformance.rs` needs nothing for B23: `Orchestrate::{store, clock, restarted}`
(`:84`, `:123`, `:138`) answer the `FakeOrchestrator`'s fields.

#### 2.6 T7 tests

In `mem.rs`'s `mod tests` (`#[cfg(all(test, feature = "test-support"))]`, `:6239`):

```rust
    /// MOD-40 plan D11 (T7): a `MemStore` stamps with the clock its handle was given, and a
    /// clone given another clock stamps with that one. Without a clock it stamps the wall clock,
    /// untruncated (blueprint B21).
    #[tokio::test]
    async fn a_mem_store_reads_its_clock() {
        let clock = TestClock::at(Utc::now() - TimeDelta::days(3));
        let store = MemStore::demo().with_clock(Arc::new(clock.clone()));
        clock.advance(TimeDelta::minutes(7));
        let project = store.project(ids::PROJECT_HTUI).await.expect("read").expect("fixture");
        let edited = /* `update_project` with a one-field patch and `project.updated_at` as the
                        token, the `ProjectPatch` shape `conformance.rs` uses */;
        assert_eq!(edited.updated_at, clock.now(), "the write is stamped by the handle's clock");

        let later = store.handle_at(clock.now() + TimeDelta::hours(1));
        let again = /* the same edit through `later`, token `edited.updated_at` */;
        assert_eq!(
            again.updated_at,
            clock.now() + TimeDelta::hours(1),
            "a handle's clock is its own; the rows are shared"
        );
        assert_eq!(
            store.project(ids::PROJECT_HTUI).await.expect("read").expect("fixture").updated_at,
            again.updated_at,
            "both handles read one set of rows"
        );

        let wall = MemStore::demo();
        let before = Utc::now();
        let stamped = /* the same edit through `wall` */;
        assert!(stamped.updated_at >= before, "no clock: the wall clock");
    }
```

The implementer picks the one-field edit from the ones `conformance.rs` already drives through
`MemStore` (`update_project`'s `ProjectPatch { description: Some(..), ..Default::default() }` is
the smallest; any writer whose answer carries `updated_at` pins the same thing). No assertion of
the untruncated nanoseconds: `Utc::now()` can land on a whole microsecond.

T7 changes no other assertion anywhere: every orch case now reads lease expiry and stamps from
its own process's clock, which is exactly the `now` each engine passed before (B23), and every
unclocked store stamps as before (B21).

---
### 3. T8 — database time for leases (D10, B25-B28): production code

#### 3.1 `htui-core/src/store/traits.rs`

`use chrono::{DateTime, TimeDelta, Utc};` (`:32`). Beside the other shared refusals
(`references_no_row`, `:1800-1804`):

```rust
/// The longest lease TTL a store accepts: 365 days, the bound `htui-orch`'s `LeaseTimes::from_app`
/// already clamps `lease_ttl_seconds` to, so `now + ttl` never overflows an instant and the µs
/// count Postgres multiplies stays below 2^53, where a `float8` product is exact (MOD-40 blueprint
/// P-13, B27).
pub const MAX_LEASE_TTL: TimeDelta = TimeDelta::seconds(365 * 24 * 60 * 60);

/// MOD-40 plan D10 (blueprint B27): a lease TTL as the whole microseconds both stores add to their
/// own clock, or the one refusal both give.
///
/// Sub-microsecond parts are dropped (`TIMESTAMPTZ` has none), so Postgres, which binds the
/// count, and `MemStore`, which adds `TimeDelta::microseconds(count)`, add the same span.
///
/// # Errors
/// [`StoreError::Constraint`](crate::store::StoreError::Constraint) naming the TTL when it is
/// negative or longer than [`MAX_LEASE_TTL`]. Checked before any row is read, so an out-of-range
/// TTL on an unknown run is this refusal, not `NotFound`.
pub fn lease_ttl_micros(ttl: TimeDelta) -> crate::store::Result<i64> {
    match ttl.num_microseconds() {
        Some(micros) if ttl >= TimeDelta::zero() && ttl <= MAX_LEASE_TTL => Ok(micros),
        _ => Err(crate::store::StoreError::Constraint(format!(
            "lease ttl {ttl} is outside 0 ..= {MAX_LEASE_TTL}"
        ))),
    }
}
```

`htui-core/src/store/mod.rs:15-29`: add `MAX_LEASE_TTL` and `lease_ttl_micros` to the
`pub use traits::{…}` list (alphabetical: `MAX_LEASE_TTL` after `GLOB_NEEDS_GLOBS`,
`lease_ttl_micros` after `item_not_in_project`).

The five signatures and their docs (`:932-1047`), whole replacements. `claim_run`'s long first
paragraphs (`:932-962`) are unchanged up to "On [`Claim::Admitted`]"; from there:

```rust
    /// On [`Claim::Admitted`] the run moves `queued -> running` with `executing_box_id = box_id`,
    /// `started_at = at`, `lease_box_id = box_id`, `lease_owner = owner`, and `lease_expires_at`
    /// the **store's** clock plus `ttl` (MOD-40 plan D10: Postgres's `clock_timestamp()`), and the
    /// item `queued -> in_progress`. On [`Claim::MissingTags`] the run moves `queued -> failed`
    /// with `failure` = [`missing_tags_failure`](crate::model::missing_tags_failure) of the list
    /// and `finished_at = at`, and its item `queued -> blocked`, in the same transaction;
    /// `executing_box_id`, `started_at` and the lease stay unset, so no slot is taken. Every other
    /// answer writes nothing.
    ///
    /// `at` is the caller's clock, like every other stamp of the run's timeline; only the lease is
    /// the store's, because only the lease is compared by another process (MOD-40 blueprint B25).
    ///
    /// The two predicates range over two different sets, and `awaiting_approval` is where they
    /// part: a parked run consumes no compute and so holds no slot, but it still owns its trees
    /// and its unmerged branch and so still refuses an overlapping scope (§4.7, invariant 6).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` outside
    /// [`lease_ttl_micros`]'s range, before anything is read;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) for an unknown run (`"run"`)
    /// or box (`"box"`), the run looked up first.
    async fn claim_run(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        at: DateTime<Utc>,
        ttl: TimeDelta,
    ) -> Result<Claim>;

    /// ANA-2 §4.9's heartbeat: `UPDATE run SET lease_expires_at = <store now> + ttl WHERE id = run
    /// AND lease_owner = owner`. `Ok(false)` = zero rows = abandon; the run exists but is not
    /// ours. The expiry is the store's clock (MOD-40 plan D10) and is not returned: the caller
    /// fences on its own clock, from the instant it sent the refresh (plan OQ-2).
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`.
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool>;

    /// ANA-2 §4.9's sweep: every `running` run whose `executing_box_id` is `box_id` and whose
    /// lease is `NULL` or expired **by the store's clock** becomes ours (`lease_owner = owner`,
    /// `lease_expires_at` = the store's clock plus `ttl`). Returns the adopted rows in `queued_at`
    /// order, ties broken by `id` so the order is total and the same on every backend; empty when
    /// nothing was abandoned. A box that does not exist adopts nothing (`Ok(vec![])`).
    ///
    /// **Never** a run whose `lease_owner` is `owner` (plan D88): a process whose heartbeat
    /// stalled past its TTL must not adopt its own live walk and run it twice under one owner.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// the backend's own failures.
    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>>;

    /// Plan D87: the lease of a run that is ours or free, taken before a command's first write on
    /// a parked or running run. `UPDATE run SET lease_owner = owner, lease_box_id = box_id,
    /// lease_expires_at = <store now> + ttl WHERE id = run AND status IN
    /// ('running','awaiting_approval') AND executing_box_id = box_id AND (lease_owner = owner OR
    /// lease_owner IS NULL OR lease_expires_at IS NULL OR lease_expires_at <= <store now>)`.
    ///
    /// `Ok(false)` = zero rows: another owner holds a lease live by the store's clock, or the run
    /// is not takeable here (not `running`/`awaiting_approval`, or executing on another box).
    /// Nothing is written then.
    ///
    /// # Errors
    /// [`StoreError::Constraint`](crate::store::StoreError::Constraint) for a `ttl` out of range;
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, told
    /// apart from "not takeable" by one follow-up read.
    async fn take_lease(
        &self,
        run: RunId,
        box_id: BoxId,
        owner: Uuid,
        ttl: TimeDelta,
    ) -> Result<bool>;

    /// Plan D139: gives a lease back. `UPDATE run SET lease_owner = NULL, lease_expires_at =
    /// <store now> WHERE id = run AND lease_owner = owner`. `Ok(false)` = zero rows = not ours,
    /// and nothing is written.
    ///
    /// (the four paragraphs `:1037-1050` from "The owner is cleared, not only the expiry." to the
    /// link block, unchanged)
    ///
    /// # Errors
    /// [`StoreError::NotFound`](crate::store::StoreError::NotFound) `{ entity: "run" }`, told
    /// apart from "not ours" by one follow-up read.
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool>;
```

(`release_lease` takes no TTL, so it has no `Constraint` case.)

#### 3.2 `htui-store/src/pg/write.rs` — the SQL (P-12, P-13, P-14)

`use chrono::{DateTime, TimeDelta, Utc};` (`:21`); `lease_ttl_micros` joins the
`htui_core::store::{…}` import (`:42-51`). Each method's first line is
`let ttl = lease_ttl_micros(ttl)?;` (an `i64` from there on), except `release_lease`. The
follow-up `SELECT 1 FROM run WHERE id = $1` reads, the `NotFound` answers and every doc paragraph
not named here are unchanged.

- **`claim_run`** (`:3533-3720`): signature `(…, at: DateTime<Utc>, ttl: TimeDelta)`. The
  `MissingTags` `UPDATE` (`:3598-3610`) is **byte-identical** (still `$3 = at`, so its `.sqlx`
  file stays). The admitted `UPDATE` (`:3690-3706`):

  ```rust
        sqlx::query!(
            "UPDATE run \
                SET status           = 'running', \
                    executing_box_id = $2, \
                    started_at       = COALESCE(started_at, $4), \
                    lease_box_id     = $2, \
                    lease_owner      = $3, \
                    lease_expires_at = clock_timestamp() + $5::bigint * interval '1 microsecond' \
              WHERE id = $1 AND status = 'queued'",
            run.as_uuid(),
            box_id.as_uuid(),
            owner,
            at,
            ttl,
        )
  ```

  Doc (`:3508-3532`) gains one paragraph before its lock paragraph: "The lease's expiry is
  `clock_timestamp()` plus the TTL, in the statement that admits the run, so no process's clock
  enters a lease (MOD-40 plan D10); `started_at` and a `MissingTags` `finished_at` are the
  caller's `at` (blueprint B25). The product `$5 * interval '1 microsecond'` keeps the span in the
  interval's time field, so the sum is absolute in any session `TimeZone` (P-13)."
- **`refresh_lease`** (`:3735`):

  ```rust
    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        let ttl = lease_ttl_micros(ttl)?;
        let moved = sqlx::query!(
            "UPDATE run SET lease_expires_at = clock_timestamp() + $3::bigint * interval '1 microsecond' \
              WHERE id = $1 AND lease_owner = $2",
            run.as_uuid(),
            owner,
            ttl,
        )
  ```

  (the implementer breaks the string where rustfmt's 100 columns need it; the query text sqlx
  hashes is whatever the literal concatenates to, so keep one space at each `\` join.)
- **`adopt_runs`** (`:3781-3845`): signature `(&self, box_id: BoxId, owner: Uuid, ttl:
  TimeDelta)`. In the CTE, `AND (lease_expires_at IS NULL OR lease_expires_at <= $3)` becomes
  `<= clock_timestamp())`, and `lease_expires_at = $4` becomes `lease_expires_at =
  clock_timestamp() + $3::bigint * interval '1 microsecond'`; the binds are `box_id.as_uuid(),
  owner, ttl`. The `SELECT … FROM swept ORDER BY queued_at, id` list is unchanged. Doc: "expired
  at `now`" → "expired by `clock_timestamp()`".
- **`take_lease`** (`:3847-3900`):

  ```rust
        let moved = sqlx::query!(
            "UPDATE run \
                SET lease_owner      = $3, \
                    lease_box_id     = $2, \
                    lease_expires_at = clock_timestamp() + $4::bigint * interval '1 microsecond' \
              WHERE id = $1 \
                AND status IN ('running','awaiting_approval') \
                AND executing_box_id = $2 \
                AND (lease_owner = $3 OR lease_owner IS NULL \
                     OR lease_expires_at IS NULL OR lease_expires_at <= clock_timestamp())",
            run.as_uuid(),
            box_id.as_uuid(),
            owner,
            ttl,
        )
  ```

  Under READ COMMITTED a take that waits on another's row lock re-evaluates its `WHERE` against
  the committed row, and `clock_timestamp()` is volatile, so the re-check reads the clock after
  the wait (the race `pg_criteria.rs::two_takes_of_one_released_lease_admit_one` pins).
- **`release_lease`** (`:3906`):

  ```rust
    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        let moved = sqlx::query!(
            "UPDATE run SET lease_owner = NULL, lease_expires_at = clock_timestamp() \
              WHERE id = $1 AND lease_owner = $2",
            run.as_uuid(),
            owner,
        )
  ```

`clock_timestamp()`, not `now()`: `now()` is the transaction's start, which in `claim_run` is
before the box row lock is waited on (`:3561-3571`), so a claim that queued behind another would
write a lease shorter than its TTL; and it is the precedent `register_box` and `touch_box` set
(`pg/mod.rs:460`, m2-m3 blueprint §5.1). Every write reads the clock after the local send
instant, which is the property the heartbeat's fence needs (F-38).

#### 3.3 `htui-core/src/store/mem.rs`

Inner fns (`:3893-4113`), the signatures and the lines that change:

```rust
    fn claim_run(&mut self, run: RunId, box_id: BoxId, owner: Uuid, at: DateTime<Utc>,
                 ttl: TimeDelta, now: DateTime<Utc>) -> Result<Claim>
        // :3994  row.lease_expires_at = Some(now + ttl);          (was Some(lease_until))

    fn refresh_lease(&mut self, run: RunId, owner: Uuid, ttl: TimeDelta, now: DateTime<Utc>)
        -> Result<bool>
        // :4023  row.lease_expires_at = Some(now + ttl);          (was Some(until))

    fn adopt_runs(&mut self, box_id: BoxId, owner: Uuid, ttl: TimeDelta, now: DateTime<Utc>)
        -> Vec<Run>
        // :4045  && row.lease_expires_at.is_none_or(|until| until <= now)   (was `<= at`)
        // :4058  row.lease_expires_at = Some(now + ttl);          (was Some(lease_until))

    fn take_lease(&mut self, run: RunId, box_id: BoxId, owner: Uuid, ttl: TimeDelta,
                  now: DateTime<Utc>) -> Result<bool>
        // :4086  … row.lease_expires_at.is_none_or(|expiry| expiry <= now)  (was `<= at`)
        // :4092  row.lease_expires_at = Some(now + ttl);          (was Some(until))

    fn release_lease(&mut self, run: RunId, owner: Uuid, now: DateTime<Utc>) -> Result<bool>
        // :4108  row.lease_expires_at = Some(now);                (was Some(at))
```

`now` is the one instant the wrapper read, so a lease and the row's `updated_at` are stamped
alike (B29). Docs: `adopt_runs`' "(plan D88)" line gains "Expiry is judged by this handle's
clock (MOD-40 plan D10)."

Wrappers (`:5957-6004`):

```rust
    async fn claim_run(&self, run: RunId, box_id: BoxId, owner: Uuid, at: DateTime<Utc>,
                       ttl: TimeDelta) -> Result<Claim> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.claim_run(run, box_id, owner, at, ttl, now))
    }

    async fn refresh_lease(&self, run: RunId, owner: Uuid, ttl: TimeDelta) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::RefreshLease)?;
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.refresh_lease(run, owner, ttl, now))
    }

    async fn adopt_runs(&self, box_id: BoxId, owner: Uuid, ttl: TimeDelta) -> Result<Vec<Run>> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        Ok(self.write(|state| state.adopt_runs(box_id, owner, ttl, now)))
    }

    async fn take_lease(&self, run: RunId, box_id: BoxId, owner: Uuid, ttl: TimeDelta)
        -> Result<bool> {
        let ttl = TimeDelta::microseconds(lease_ttl_micros(ttl)?);
        let now = self.now();
        self.write(|state| state.take_lease(run, box_id, owner, ttl, now))
    }

    async fn release_lease(&self, run: RunId, owner: Uuid) -> Result<bool> {
        #[cfg(feature = "test-support")]
        self.check_fault(MemFault::ReleaseLease)?;
        let now = self.now();
        self.write(|state| state.release_lease(run, owner, now))
    }
```

(rustfmt lays the signatures out.) The fault check stays first ("before it touches any state",
`:67-75`). `mem.rs` imports `TimeDelta` from chrono and `lease_ttl_micros` from
`crate::store::traits` (`:44-56`). A test-support `MemStore` in a harness with a frozen
`TestClock` that writes `ttl = 0` and then adopts at the same instant finds the lease expired:
`<=`, as Postgres (P-12).

#### 3.4 `Writer` and the test doubles (forward only)

- `htui-store/src/writer.rs:807-860`: the five forwards take and pass the new parameters, both
  arms; `use chrono::{DateTime, TimeDelta, Utc};` (`:23`).
- `htui-agent/src/conformance.rs:1026-1072` (`UsageSpy`) and `htui-agent/tests/recorder.rs:767-813`
  (`SpyStore`): the same signatures, forwarding to `self.inner`. `conformance.rs:30` adds
  `TimeDelta` to its chrono import; `recorder.rs:20` already has it.

#### 3.5 `htui-orch`: the engine and the heartbeat (B26, B28)

**`recover.rs`**

- Module doc `:3-5`: "the engine hands it `|until| store.refresh_lease(run, owner, until)`" →
  "`|ttl| store.refresh_lease(run, owner, ttl)`".
- `:37-38`: `const MAX_LEASE_TTL_SECONDS: i64 = htui_core::store::MAX_LEASE_TTL.num_seconds();`
  with its doc gaining "the store's own bound (MOD-40 blueprint B27)". `from_app` is unchanged.
- `LeaseTimes::ttl`'s doc (`:44-45`): "Added to the clock's instant for every `lease_expires_at`
  this process writes." → "The TTL this process hands every lease write. The store adds it to its
  own clock (MOD-40 plan D10); the heartbeat adds it to the local instant it sent the write at,
  which is where its self-fence is measured from."
- `heartbeat` (`:104-160`). Signature: `F: FnMut(TimeDelta) -> Fut`. Body: `refresh(until)` at
  `:141` becomes `refresh(times.ttl)`; nothing else changes (`until = now + times.ttl` stays and
  still moves the fence on `Ok(true)`). Doc, whole second paragraph replaced:

  ```rust
/// Sleep, then `refresh(times.ttl)`: `Ok(true)` → the lease runs at least to `clock.now() +
/// times.ttl` as read **before** the refresh was sent, sleep `times.refresh`, loop; `Ok(false)` →
/// `Abandoned`; `Err(e)` → `tracing::warn!`, sleep `times.refresh / 4` (at least 1 s, D123) and
/// loop (D86, D103). Never returns otherwise.
///
/// **The self-fence (plan D122; MOD-40 plan OQ-2).** The store stamps a lease with its own clock
/// and does not say what it wrote (plan D10). The heartbeat instead tracks, on its **own** clock,
/// the instant by which the last lease it wrote successfully lapses at the earliest: the instant
/// it sent that write plus the TTL. The store read its clock after that send, so its expiry is no
/// earlier, and a span measured on one clock does not care how far apart two hosts' clocks are.
/// It starts from `written`, the same instant for the caller's `claim_run`, `take_lease` or the
/// sweep's `renew_lease` (plan D143), which may lie before `clock.now() + times.ttl` at this call
/// when the caller wrote other rows in between. Once refreshes have failed until
/// `clock.now() >= last - times.refresh`, it returns `Expired` … (the rest of the paragraph,
/// `:115-121`, unchanged)
///
/// The clock is the wall clock in production, so a wall-clock step while a lease is held moves
/// the fence with it (MOD-40 blueprint F-38); a monotonic clock is MOD-41's.
  ```

**`engine.rs`**

- `claim` (`:686-695`):

  ```rust
        let now = self.now();
        let ttl = self.lease_times().ttl;
        …
            .claim_run(run, self.parts.box_id, self.parts.owner, now, ttl)
        …
        let rest = self.walk_leased(run, now + ttl, self.run_to_rest(run)).await?;
  ```

  (`lease` is gone; `now` is read before the store call, which is what makes `now + ttl` a
  fence the store's expiry cannot precede, F-38.)
- `walk_leased`'s doc (`:1651-1653`): "`until` is the `lease_expires_at` the caller's lease take
  wrote (`claim_run`, `take_lease` or the sweep's renewal)." → "`until` is the local instant the
  caller's lease take (`claim_run`, `take_lease` or the sweep's renewal) lapses by at the
  earliest: the instant it was sent plus the TTL (MOD-40 plan D10, OQ-2)."
- `heartbeaten` (`:1714`): `|until| store.refresh_lease(run, owner, until)` → `|ttl|
  store.refresh_lease(run, owner, ttl)`.
- `take_lease` (`:1759-1789`), B26, whole body after the status refusal:

  ```rust
        // MOD-40 blueprint B26: no clock is read. A take on a `running` or parked run is refused
        // for one of two reasons only: the run executes on another box, or another owner's lease
        // had not lapsed by the store's clock. The expiry is the store's, and comparing it with
        // this process's clock would reintroduce the skew D10 removed.
        if row.executing_box_id == Some(self.parts.box_id) {
            return Err(EngineError::LeaseHeld { run });
        }
        Err(EngineError::RunStatus {
            run,
            status: row.status,
            expected: "running | awaiting_approval, executing on this box",
        })
  ```

  Doc (`:1759-1768`): "`take_lease(run, box, owner, now, now + ttl)`" → "`take_lease(run, box,
  owner, ttl)`"; "Answers the `until` it wrote" → "Answers the local instant its lease lapses by
  at the earliest (the send instant plus the TTL)"; the D130 sentence becomes "a run on this box
  → [`EngineError::LeaseHeld`] (the store's only remaining refusal is a live foreign lease; never
  this process's, which the take would have renewed); a run on another box →
  [`EngineError::RunStatus`] saying so, whatever its lease (MOD-40 blueprint B26)."
- `renew_lease` (`:1791-1814`):

  ```rust
        let now = self.now();
        let ttl = self.lease_times().ttl;
        let taken = self
            .parts
            .store
            .take_lease(run, self.parts.box_id, self.parts.owner, ttl)
            .await?;
        …
        Ok(taken.then_some(now + ttl))
  ```

  Doc: "The store's `take_lease(run, box, owner, now, now + ttl)` and its bare answer: the `until`
  written when the lease was taken" → "The store's `take_lease(run, box, owner, ttl)` and its bare
  answer: the local instant the lease lapses by at the earliest, `now + ttl` with `now` read before
  the take".
- `fresh_until` (`:1848-1852`, `#[cfg(test)]`): body unchanged; doc "what a test hands
  [`Self::walk_leased`] as the lease its caller has just written" stays true.
- `release_lease` (`:1868-1896`): `.release_lease(run, self.parts.owner, self.now())` →
  `.release_lease(run, self.parts.owner)`; doc "the store's `release_lease(run, owner, now)`. The
  lease reads as expired at once" → "the store's `release_lease(run, owner)`. The lease reads as
  expired at once, by the store's clock".
- `sweep_fenced` (`:1961-1968`): `let now = self.now();` is deleted; `.adopt_runs(self.parts.box_id,
  self.parts.owner, now, now + times.ttl)` → `.adopt_runs(self.parts.box_id, self.parts.owner,
  times.ttl)`.

No other production file names a lease method (`grep -rn '\.\(claim_run\|refresh_lease\|adopt_runs\|take_lease\|release_lease\)(' crates/*/src`
minus the test modules = the sites above plus the five implementors and `Writer`).

**`htui/src/run_worker.rs`**: no production change (it calls `Engine`, never a lease method).

---
### 4. T8 — new tests and the shared helper

#### 4.0 `assert_leased_for` (B29), `htui-core/src/store/conformance.rs`

Public (the module is `test-support` already, `store/mod.rs:11`), placed after `run_row`
(`:4175`), so `pg_criteria.rs` and the orch tests can use it:

```rust
/// MOD-40 blueprint B29: `run`'s lease was written for `ttl` by the write that last stamped the
/// row. A lease is the store's clock plus a TTL (plan D10), and a generic case cannot read that
/// clock, so the expiry is read against the row's own `updated_at`, which the same write stamped
/// on the same clock: `MemStore` stamps both from one read (the span is exactly `ttl`); Postgres
/// stamps `updated_at` in its `BEFORE UPDATE` trigger, a few µs after the `SET` read
/// `clock_timestamp()` (the span is `ttl` less that gap). A released lease is `ttl = 0`.
///
/// Holds only while the lease write is the row's last write; read the row right after it.
///
/// # Panics
/// When the run holds no lease or the span is outside `(ttl - 1 s, ttl]`.
pub fn assert_leased_for(case: &str, run: &Run, ttl: TimeDelta, what: &str) {
    let Some(expiry) = run.lease_expires_at else {
        panic!("{case}: {what}: the run holds no lease");
    };
    let span = expiry - run.updated_at;
    assert!(
        span <= ttl && span > ttl - TimeDelta::seconds(1),
        "{case}: {what}: the lease runs {span} past the write that stamped it, not {ttl}"
    );
}
```

#### 4.1 Store case `a_lease_ttl_out_of_range_is_refused` (B27, B30)

`CASES` gains `"a_lease_ttl_out_of_range_is_refused",` after
`"upsert_agent_with_the_current_token_applies",`; `run_case` gains its arm before `other =>`;
the body goes after that case's body. `use crate::store::{MAX_LEASE_TTL, …}` in the module's
imports.

```rust
/// MOD-40 blueprint B27: a lease TTL below zero or past [`MAX_LEASE_TTL`] is `Constraint` from
/// every method that takes one, checked before any row is read (an unknown run is the same
/// refusal, not `NotFound`), and writes nothing. Both bounds are accepted.
async fn a_lease_ttl_out_of_range_is_refused<S: WriteStore>(store: &S) {
    const CASE: &str = "a_lease_ttl_out_of_range_is_refused";
    let owner = Uuid::now_v7();
    let at = seam_clock();
    let out_of_range = [
        TimeDelta::microseconds(-1),
        MAX_LEASE_TTL + TimeDelta::microseconds(1),
    ];
    let refused = |answer: Result<_>, what: &str| {
        assert!(
            matches!(answer, Err(StoreError::Constraint(_))),
            "{CASE}: {what} is Constraint, got {answer:?}"
        );
    };
    let run = store
        .create_run(new_run(ids::PROJECT_HTUI, ids::HTUI_ANA_2, Vec::new()))
        .await
        .expect(CASE)
        .id;
    let queued = run_row(CASE, store, run).await;
    for ttl in out_of_range {
        refused(store.claim_run(run, ids::BOX, owner, at, ttl).await.map(|_| ()), "a claim");
        refused(
            store.claim_run(RunId::new(), ids::BOX, owner, at, ttl).await.map(|_| ()),
            "a claim of an unknown run",
        );
    }
    assert_eq!(run_row(CASE, store, run).await, queued, "{CASE}: a refused claim writes nothing");

    assert_eq!(
        store.claim_run(run, ids::BOX, owner, at, MAX_LEASE_TTL).await.expect(CASE),
        Claim::Admitted,
        "{CASE}: the longest TTL is admitted"
    );
    let claimed = run_row(CASE, store, run).await;
    assert_leased_for(CASE, &claimed, MAX_LEASE_TTL, "the longest lease");
    for ttl in out_of_range {
        refused(store.refresh_lease(run, owner, ttl).await.map(|_| ()), "a refresh");
        refused(store.refresh_lease(RunId::new(), owner, ttl).await.map(|_| ()), "a refresh of an unknown run");
        refused(store.take_lease(run, ids::BOX, owner, ttl).await.map(|_| ()), "a take");
        refused(store.adopt_runs(ids::BOX, Uuid::now_v7(), ttl).await.map(|_| ()), "a sweep");
    }
    assert_eq!(run_row(CASE, store, run).await, claimed, "{CASE}: the refusals wrote nothing");
    assert!(
        store.refresh_lease(run, owner, TimeDelta::zero()).await.expect(CASE),
        "{CASE}: a zero TTL is accepted"
    );
    assert_leased_for(CASE, &run_row(CASE, store, run).await, TimeDelta::zero(), "a zero lease");
}
```

(`Result` is `crate::store::Result`, already imported; `Run: PartialEq`. rustfmt lays it out.)
Red on commit (1) because neither store gates yet: MemStore answers the negative claim
`Admitted`; Pg's red half computes `Utc::now() + ttl`.

**Pins**: `htui-core/tests/mem_store.rs:37` 95 → **96**, its message gains "and one for the lease
TTL's range (MOD-40 plan D10)"; `htui-store/tests/pg_conformance.rs:19` 95 → **96**.

#### 4.2 `htui-store/tests/pg_criteria.rs::lease_times_are_the_databases` (D10's pin)

After `two_takes_of_one_released_lease_admit_one`. Runtime `sqlx::query` / `query_scalar` only,
so `.sqlx` does not grow.

```rust
/// MOD-40 plan D10 (C2): every lease a lease method writes, and every expiry one compares, is
/// the database's `clock_timestamp()`. The caller's clock is a day slow and enters nothing but
/// `started_at` (blueprint B25). Each expiry lies between two `clock_timestamp()` reads taken
/// around its write, plus the TTL.
#[tokio::test(flavor = "multi_thread")]
async fn lease_times_are_the_databases() {
    let Some(db) = common::demo_db().await else {
        return;
    };
    let (a, b) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    let slow = (Utc::now() - TimeDelta::days(1)).trunc_subsecs(TIMESTAMPTZ_DIGITS);
    let db_now = || async {
        sqlx::query_scalar::<_, DateTime<Utc>>("SELECT clock_timestamp()")
            .fetch_one(&db.pool)
            .await
            .expect("read the database's clock")
    };
    let row = || async {
        db.store.run(ids::RUN_2).await.expect("read must not fail").expect("the run exists")
    };
    let bracketed = |lease: Option<DateTime<Utc>>, from: DateTime<Utc>, to: DateTime<Utc>,
                     ttl: TimeDelta, what: &str| {
        let lease = lease.unwrap_or_else(|| panic!("{what}: no lease"));
        assert!(
            from + ttl <= lease && lease <= to + ttl,
            "{what}: {lease} is not the database's clock plus {ttl} (read {from} .. {to})"
        );
    };

    let from = db_now().await;
    assert_eq!(
        db.store.claim_run(ids::RUN_2, ids::BOX, a, slow, TimeDelta::minutes(5)).await
            .expect("the claim must not fail"),
        Claim::Admitted,
        "A claims the queued fixture run"
    );
    let to = db_now().await;
    let claimed = row().await;
    assert_eq!(claimed.started_at, Some(slow), "started_at is the caller's clock (B25)");
    bracketed(claimed.lease_expires_at, from, to, TimeDelta::minutes(5), "the claim");

    let from = db_now().await;
    assert!(db.store.refresh_lease(ids::RUN_2, a, TimeDelta::minutes(10)).await.expect("refresh"));
    bracketed(row().await.lease_expires_at, from, db_now().await, TimeDelta::minutes(10), "the refresh");

    let from = db_now().await;
    assert!(db.store.take_lease(ids::RUN_2, ids::BOX, a, TimeDelta::minutes(15)).await.expect("take"));
    bracketed(row().await.lease_expires_at, from, db_now().await, TimeDelta::minutes(15), "the renewal");

    let from = db_now().await;
    assert!(db.store.release_lease(ids::RUN_2, a).await.expect("release"));
    bracketed(row().await.lease_expires_at, from, db_now().await, TimeDelta::zero(), "the release");

    let from = db_now().await;
    let adopted = db.store.adopt_runs(ids::BOX, b, TimeDelta::minutes(7)).await.expect("adopt");
    assert_eq!(adopted.iter().map(|r| r.id).collect::<Vec<_>>(), [ids::RUN_2], "B's sweep adopts the released run");
    bracketed(row().await.lease_expires_at, from, db_now().await, TimeDelta::minutes(7), "the adoption");

    // Expiry is compared on the database's clock too: a lease planted 1 s in the past is a
    // stranger's to take, one planted an hour ahead is not, whatever the caller's clock says.
    let plant = |offset: &'static str, owner: uuid::Uuid| {
        let pool = db.pool.clone();
        async move {
            sqlx::query(&format!(
                "UPDATE run SET lease_owner = $1, lease_expires_at = clock_timestamp() + interval '{offset}' WHERE id = $2"
            ))
            .bind(owner)
            .bind(ids::RUN_2.as_uuid())
            .execute(&pool)
            .await
            .expect("plant a lease");
        }
    };
    plant("-1 second", b).await;
    assert!(db.store.take_lease(ids::RUN_2, ids::BOX, a, TimeDelta::minutes(5)).await.expect("take"),
        "a lease lapsed by the database's clock is taken");
    plant("1 hour", b).await;
    assert!(!db.store.take_lease(ids::RUN_2, ids::BOX, a, TimeDelta::minutes(5)).await.expect("take"),
        "a lease live by the database's clock is refused");
    assert!(db.store.adopt_runs(ids::BOX, a, TimeDelta::minutes(5)).await.expect("adopt").is_empty(),
        "and not swept");

    db.drop_db().await;
}
```

(The implementer inlines the closures if borrowck objects; the assertions are the contract.)

Red on commit (1): there `PgStore::claim_run` still writes the caller-derived expiry `at + ttl`
(the old statement, `at` standing for the old `lease_until - ttl`), so the claim's lease is
`slow + 5 min`, a day before the bracket. The other brackets could pass by accident when the
test host is the database host (`Utc::now() + ttl` lands inside them), which is why the claim,
with its deliberately slow caller, is the red that counts.

#### 4.3 Engine `a_skewed_engine_still_names_a_live_lease_held` (B26)

`htui-orch/src/engine.rs` tests, after `resume_on_another_box_s_run_with_a_lapsed_lease_is_not_lease_held`
(`:10037-10088`):

```rust
    /// MOD-40 blueprint B26: a live lease another process holds is `LeaseHeld` whatever this
    /// process's clock says. The engine's clock is two days ahead of the store's, so the old
    /// check (`lease_expires_at > self.now()`) read the stranger's one-day lease as lapsed and
    /// answered `RunStatus`; the store refused the take, and the run executes here, so the
    /// lease is held.
    #[tokio::test]
    async fn a_skewed_engine_still_names_a_live_lease_held() {
        let harness = Harness::new().await;
        let (run, _) = started(&harness).await;
        let other = a_live_stranger(&harness, run).await;
        let before = other.orch.run(run).await;
        let skewed = crate::fake::TestClock::at(other.orch.clock.now() + TimeDelta::days(2));

        let graphs = other.orch.graphs();
        let driver =
            |_candidate: &SnapshotCandidate, key: &SessionKey<'_>| other.orch.driver_for_key(key);
        let scrubber = htui_core::scrub::MinimalScrubber::new([]);
        let mut parts = super::fake_parts(&other.orch, &graphs, &driver, &scrubber)
            .await
            .expect("the harness has a box");
        parts.clock = &skewed;
        let engine = super::Engine::new(parts);

        let refused = engine
            .take_lease(run)
            .await
            .expect_err("the first process's lease is live by the store's clock");
        assert!(
            matches!(refused, EngineError::LeaseHeld { run: named } if named == run),
            "{refused}"
        );
        assert_eq!(other.orch.run(run).await, before, "the refused take wrote nothing");
    }
```

The store handle in `parts` is `other.orch.store`, clocked on `other`'s own `TestClock` (B23),
so the store judges the lease live (`harness` took it until `harness.now + 1 day`, `other` reads
`harness.now + 10 min`); only the engine's clock is skewed. Red on the old `:1781`.

#### 4.4 T7's tests

§2.6 (unchanged here).

---
### 5. T8 — test rewrites, case by case (meaning unchanged)

Lines at `cd49a3b`; `@main` = `origin/main` `ca2fb1f` for the PR #13 cases (§10). Three
techniques, named per case:

- **(L) live**: a TTL of minutes or days, as before.
- **(E) expired**: the holder writes `ttl = 0` (a `refresh_lease` by the holder, or a claim with
  `ttl = 0`); the lease is then lapsed by `<=` at once on both stores (P-12). Replaces every
  "pass a `now` past the lease".
- **(C) clocked**: on a `MemStore` handle clocked by a `TestClock` (mem.rs unit tests, the orch
  harness), `clock.set(old_now)` then `ttl = old_until - old_now` reproduces the old instant
  exactly, so an exact assertion stays exact. **(H)**: `store.handle_at(old_now)` for a stranger
  at another instant than the case's own clock (B24).

A value assertion on a generic store becomes `assert_leased_for` (B29); a "refused write writes
nothing" assertion on one column becomes the whole row (`run_row(…) == before`).

#### 5.1 `htui-core/src/store/conformance.rs` (generic; techniques L, E)

Module level: `const LEASE: TimeDelta = TimeDelta::minutes(5);` beside `seam_clock()` (`:4056`),
doc "The TTL the lease cases claim with: live for the whole case on any store (MOD-40 plan D10)."
`use crate::store::MAX_LEASE_TTL` (§4.1). Every `let until = at + TimeDelta::minutes(5);` that
only feeds `claim_run` is deleted and the claim passes `LEASE`.

| Case (line) | Old | New |
|---|---|---|
| `claim_run_admits_one_run_per_slot…` `:4364` | `until = at + 5 min`; seven claims `(at, until)` `:4408-4540`; `claimed.lease_expires_at == Some(until)` "the lease expires when the caller said" `:4435-4439` | claims `(at, LEASE)`; `assert_leased_for(CASE, &claimed, LEASE, "the lease runs its TTL from the store's clock")`. `started_at == Some(at)` `:4426-4430` unchanged (B25). |
| isolation rules `:4560` | `claimed_at = at + 1 min; until = claimed_at + 5 min`; closure `:4650` | `until` deleted; closure `claim_run(run, ids::BOX, owner, claimed_at, LEASE)`. No lease assertion. |
| `claim_run_fails_a_run_whose_item_needs_a_tag_the_box_lacks` `:4793` | eleven claims `(at, until)` `:4806-5101`; `failed.lease_expires_at, None` `:4872` | `(at, LEASE)`; `:4872` and every `finished_at == Some(at)` unchanged (B25). |
| `lease_refresh_is_a_cas_on_owner` `:5118` | see steps | (1) claim `(at, LEASE)`. (2) `refresh_lease(run, first_owner, 10 min)` → true; `:5146-5150` `== Some(extended)` → `assert_leased_for(…, 10 min, "the new expiry is stored")`. (3) `let before = run_row(…)`; stranger `refresh_lease(run, second_owner, 15 min)` → false; `:5159-5163` → `run_row(…) == before` "a refused heartbeat writes nothing". (4) unknown run `refresh_lease(RunId::new(), first_owner, 10 min)` → NotFound. (5) **(L)** `adopt_runs(BOX, second_owner, 15 min)` → empty "a live lease is not abandoned" (was `now = extended - 1 s`). (6) **(E)** `refresh_lease(run, first_owner, zero)` → true, comment "the first owner's lease lapses (MOD-40: a zero TTL)"; `adopt_runs(BOX, second_owner, 15 min)` → `[run]`; `:5200-5204` `== Some(swept)` → `assert_leased_for(CASE, &adopted[0], 15 min, "the adopted row carries the sweeper's expiry")`. (7) `refresh_lease(run, first_owner, 15 min)` → false; `refresh_lease(run, second_owner, 15 min)` → true. (8) **(E)** `refresh_lease(run, second_owner, zero)` → true; `adopt_runs(BOX, second_owner, 5 min)` → empty "a process never adopts its own lease, even expired (plan D88)"; `adopt_runs(BOX, first_owner, 5 min)` → `[run]`. (9) `adopt_runs(BoxId::new(), second_owner, zero)` → empty. |
| `take_lease_moves_only_our_own_or_an_expired_lease` `:5263` | `minutes = \|n\| at + n min` | `minutes = TimeDelta::minutes` (a TTL now). (1) claim `(at, minutes(5))`. (2) `let before = run_row`; Y `take(run, BOX, y, minutes(10))` → false; `:5289-5293` → `run_row == before`. (3) X `take(…, x, minutes(10))` → true; `:5301-5305` → `assert_leased_for(…, minutes(10), "the renewal stores the new expiry")`. (4) **(E)** new `refresh_lease(run, x, zero)` → true "X's lease lapses"; Y `take(…, y, minutes(9))` → true "once X's lease has expired, Y takes it". (5) `refresh(run, x, minutes(9))` → false; `refresh(run, y, minutes(9))` → true. (6) park `transition_run(…, at + 11 min)` unchanged (a caller stamp). (7) `refresh_lease(run, y, minutes(11))` "Y releases the lease at the park (lease_expires_at = now)" → `refresh_lease(run, y, TimeDelta::zero())`, same message. (8) Z `take(…, z, minutes(19))` → true; `:5346-5350` `(lease_box_id, lease_expires_at) == (Some(BOX), Some(minutes(30)))` → `taken.lease_box_id == Some(BOX)` and `assert_leased_for(…, minutes(19), "the take writes the box and the expiry")`. (9) `take(run, BoxId::new(), z, minutes(10))` → false. (10) queued `take(queued, BOX, z, minutes(5))` → false; `lease_expires_at == None` unchanged. (11) `claim_run(finished, BOX, x, at, TimeDelta::zero())` **(E)** "the second run is admitted, its lease lapsed at once"; `finish_run(…, at + 1 min)` unchanged; `take(finished, BOX, x, minutes(10))` → false "a terminal run is not takeable, even by its own expired owner". (12) `take(RunId::new(), BOX, x, minutes(5))` → NotFound. |
| `release_lease_frees_the_run_for_its_own_sweep` `:5422` | | (1) claim `(at, minutes(5))` as above. (2) `let before = run_row`; `release_lease(run, y)` → false; `:5445-5449` → `run_row == before`. (3) `refresh_lease(run, x, minutes(5))` → true. (4) `release_lease(run, x)` → true; `:5460-5464` → `released.lease_box_id == Some(BOX)` and `assert_leased_for(…, TimeDelta::zero(), "the expiry reads the store's now; the box that held the lease is kept")`. (5) `refresh(run, x, minutes(10))` → false; `release_lease(run, x)` → false. (6) `adopt_runs(BOX, x, minutes(10))` → `[run]`; `refresh(run, x, minutes(10))` → true. (7) `release_lease(RunId::new(), x)` → NotFound. |
| `leased_step` helper `:5503` | claim `(at, at + 5 min)` "until at + 5 min" | `(at, LEASE)`, message "A claims the run for five minutes". Doc: "claimed by `a` until `at + 5 min`" → "claimed by `a` for [`LEASE`]". |
| `taken_by` helper `:5531` | `take(run, BOX, b, at + 6 min, at + 20 min)` | `async fn taken_by<S: WriteStore>(case: &str, store: &S, run: RunId, a: Uuid, b: Uuid)`: **(E)** `assert!(store.refresh_lease(run, a, TimeDelta::zero()).await.expect(case), "{case}: A's lease lapses, as a suspended holder's does")`, then `assert!(store.take_lease(run, ids::BOX, b, TimeDelta::minutes(14)).await.expect(case), "{case}: B takes A's lapsed lease")`. Doc gains "A's lapse is a zero-TTL refresh (MOD-40 plan D10)". |
| `taken_by` callers | `:5626`, `:5672`, `:5907`: `taken_by(CASE, store, run, b, at)` | `taken_by(CASE, store, run, a, b)` (each case has `a` in scope). |
| `an_unleased_fence_is_refused_on_a_leased_run` `:5823` | `release_lease(run, a, at + 1 min)` | `release_lease(run, a)`. |
| `finish_run_moves_run_and_item_together` `:10692` | claims `(at, until)` `:10707`, `:10765`, `:10804` | `(at, LEASE)`; `until` deleted. `finished_at` "the caller's clock, not the store's" `:10722` unchanged. |
| seeded-run read `:11308` | `(None, None)` | unchanged. |

`claim_run_admits…` and `lease_refresh…` doc comments that say "until" (`:4360`, `:5112-5117`):
"the lease the caller asked for" → "the caller's TTL on the store's clock".

#### 5.2 `htui-core/src/store/mem.rs` unit tests (technique C)

Each test below builds `let clock = crate::clock::TestClock::at(<its old at>); let store =
MemStore::demo().with_clock(Arc::new(clock.clone()));` (`use std::sync::Arc;` and
`use crate::clock::TestClock;` in `mod tests`), so every exact assertion stays exact.

| Test (line) | Old | New |
|---|---|---|
| `a_switched_on_fault_answers_unreachable_until_switched_off` `:6265` | `release_lease(ghost, o, now)` ×2, `refresh_lease(ghost, o, now)` | `release_lease(ghost, o)` ×2, `refresh_lease(ghost, o, TimeDelta::zero())`; `let now` deleted. Unclocked (the store is `demo()`). |
| `claim_run_refuses_an_overlapping_scope_and_a_full_box` `:8130` | `at = Utc::now(); until = at + 5 min`; seven claims `(at, until)`; `:8173` `lease_expires_at == Some(until)` | clocked at `at`; claims `(at, TimeDelta::minutes(5))`; `:8173` unchanged (`at + 5 min` from the clock). `started_at == Some(at)` `:8171` unchanged. |
| `a_lease_refresh_is_a_cas_on_its_owner_and_the_sweep_adopts_it` `:8264` (at `@main` `:8405`) | the store-case shape with exact instants | clocked at `at`. claim `(at, until - at)`; `refresh(run, first, extended - clock.now())`; stranger `refresh(…, extended + 5 min - clock.now())`; unknown `refresh(…, extended - clock.now())`. Live sweep: `clock.set(extended - 1 s)`, `adopt_runs(BOX, second, swept - clock.now())` → empty. Expired sweep: `clock.set(extended + 1 s)`, `adopt_runs(BOX, second, swept - clock.now())` → `[run]`; `:8348` `== Some(swept)` unchanged. `refresh(run, first, swept - clock.now())` → false; `refresh(run, second, swept - clock.now())` → true. `clock.set(swept + 1 s)`; the two `adopt_runs(…, later - clock.now())` → empty / `[run]`. `adopt_runs(BoxId::new(), second, TimeDelta::zero())` → empty. |
| active-runs count `:9443` | claim `(at, at + 5 min)` | `(at, TimeDelta::minutes(5))`, unclocked. |
| `@main` `a_missing_tags_run_aimed_at_another_box_is_not_claimable` `:8220` | claim `(at, until)` `:8281`; `refresh_lease(run.id, owner, until)` `:8296` → false "the refusal took no lease" | `until` deleted; claim `(at, TimeDelta::minutes(5))`; refresh `(run.id, owner, TimeDelta::minutes(5))`. Unclocked. |
| `@main` `a_run_with_no_item_is_never_refused_for_tags` `:8328` | claims `(at, until)` `:8343`, `:8390`; `:8356-8360` `lease_expires_at == Some(until)` "the admitted claim wrote the lease it was handed" | clocked at `at` (`at = Utc::now()` stays the origin): `TestClock::at(at)`; claims `(at, TimeDelta::minutes(5))`; `:8356` unchanged, message "… the lease its TTL asked for". |

Added by T7 (§2.6): `a_mem_store_reads_its_clock`.

#### 5.3 `htui-store/tests/pg_criteria.rs` (techniques L, E, B29)

`use htui_core::store::conformance::assert_leased_for;`.

| Test (line) | Old | New |
|---|---|---|
| `admission_is_serialised_by_the_box_row_lock` `:684-685` | two claims `(at, until)` | `(at, TimeDelta::minutes(5))`; `until` deleted. |
| `two_sweeps_adopt_each_expired_run_once` `:760`, `:776-777` | claims `(at, at - 1 min)` "under a lease already expired"; adopts `(at, until)` | **(E)** claims `(at, TimeDelta::zero())`, message "…, under a lease that lapses at once"; adopts `(ids::BOX, Uuid::now_v7(), TimeDelta::minutes(5))`; `until` deleted. |
| `two_takes_of_one_released_lease_admit_one` `:832-878` | claim `(at, at + 5 min)`; `release_lease(run, first_owner, at)`; takes `(at, one_until)`/`(at, two_until)`; `lease_expires_at == Some(winner)` | claim `(at, TimeDelta::minutes(5))`; `release_lease(run, first_owner)`; `let (one_ttl, two_ttl) = (TimeDelta::minutes(10), TimeDelta::minutes(20))`; takes `(…, one_ttl)` / `(…, two_ttl)`; `assert_leased_for("two_takes_of_one_released_lease_admit_one", &row, if one { one_ttl } else { two_ttl }, "the stored expiry is the winner's")` (the two spans are 10 min apart, so the helper's 1 s window tells them apart). |
| `:3609` (item mirror, two claims) | `(at, until)` | `(at, TimeDelta::minutes(5))`. |
| `a_lease_take_committed_mid_write_fences_it` `:3967` | `(now, now + 5 min)` | `(now, TimeDelta::minutes(5))`. |
| `@main` `a_missing_tags_run_aimed_at_another_box_is_not_claimable` `:3687` | claim `(at, until)` `:3740`; refresh `(run.id, owner, until)` `:3756` | `until` deleted; `(at, TimeDelta::minutes(5))`; refresh `(run.id, owner, TimeDelta::minutes(5))`. |
| `@main` `a_run_with_no_item_is_never_refused_for_tags` `:3790` | `until = (at + 5 min).trunc_subsecs(6)` with the truncation comment `:3818-3822`; claim `:3825`; `:3839-3843` `== Some(until)` | the `until` and its comment are deleted; claim `(at, TimeDelta::minutes(5))`; `:3839` → `assert_leased_for("a_run_with_no_item_is_never_refused_for_tags", &claimed, TimeDelta::minutes(5), "the admitted claim wrote the lease its TTL asked for")`. |

New: `lease_times_are_the_databases` (§4.2).

#### 5.4 `htui-orch/src/conformance.rs` (per-handle clocks, B23; technique C)

`orch.store()` is clocked on `orch.clock()`, `other.store()` on `other.clock()`
(`= orch.clock() + RESTART_GAP`). `use crate::fake::RESTART_GAP;` if not in scope.

| Line | Old | New | Same instant because |
|---|---|---|---|
| `:4013`, `:4022` | `later = other.now + 1 min`; `orch.store().refresh_lease(run, orch.owner(), later)` → false | `later` deleted; `refresh_lease(run, orch.owner(), TimeDelta::minutes(1))` → false | a refused refresh writes nothing; the value never mattered. |
| `:4038-4046` | `take_lease(run, BOX, orch.owner(), orch.now, other.now + 1 day)` | `take_lease(run, BOX, orch.owner(), TimeDelta::days(1) + RESTART_GAP)` | `orch.now + 1 day + GAP = other.now + 1 day`. |
| `:4169-4175` | `other.store().refresh_lease(run, other.owner(), other.now + 1 min)` | `refresh_lease(run, other.owner(), TimeDelta::minutes(1))` | on `other`'s handle. |
| `:4777` | `orch.store().refresh_lease(live, orch.owner(), other.now + 1 day)` | `refresh_lease(live, orch.owner(), TimeDelta::days(1) + RESTART_GAP)` | as `:4038`. |
| `:4998-5006` | as `:4038` | as `:4038` | |

Every `lease_expires_at == Some(x.clock().now())` (a release by the walk on `x`'s handle) is
unchanged: the release stamps that handle's clock.

#### 5.5 `htui-orch/src/engine.rs` tests (C, H)

`orch.store` is clocked on `orch.clock` (B23), so every existing
`lease_expires_at == Some(…clock.now()…)` (`:8057`, `:8101`, `:8172`, `:8461`, `:9014`,
`:9116`, `:9157`, `:9228`, `:9588`, `:9627`, `:9771`, `:10137`, `:10152`, `:10306`, `:10322`,
`:10347`, `:10593`, `:11955`) is unchanged, and the cases that `advance` a clock to lapse a
lease (`:11929`, `:11170` below) keep doing so: the store handle reads that same clock.

| Line | Old | New |
|---|---|---|
| `leased_run` `:7836` | `claim_run(run, BOX, owner, now, now + ttl)` | `(run, BOX, owner, now, ttl)`. |
| `:8085`, `:8155`, `:8211` | `take_lease(run, BOX, owner, now, now + ttl)` | `take_lease(run, BOX, owner, ttl)` (clock at `now`). |
| `a_walk_whose_lease_is_taken…` `:8238-8246` | `harness.orch.store.adopt_runs(BOX, stranger, now + ttl + 1 s, now + ttl + 1 day)` | **(H)** `harness.orch.store.handle_at(now + ttl + TimeDelta::seconds(1)).adopt_runs(ids::BOX, stranger, TimeDelta::days(1) - TimeDelta::seconds(1))`. The walk's heartbeat still reads the unmoved harness clock (F-33), so the case still tests `Abandoned`. |
| `swept_while_a_stranger_takes_over` `:9084` | `other.orch.store.take_lease(run, BOX, stranger, later, until)` | **(H)** `other.orch.store.handle_at(later).take_lease(run, ids::BOX, stranger, TimeDelta::days(1))`; `later`, `until` stay (the fixture returns `until`, `= later + 1 day`, which the callers compare exactly). |
| `:10052` | `adopt_runs(BOX, Uuid::now_v7(), now, now)` "a stranger holds a lease that lapses at once" | `adopt_runs(ids::BOX, Uuid::now_v7(), TimeDelta::zero())`, same message. |
| `:10108-10113` | `adopt_runs(BOX, Uuid::now_v7(), now, now + 1 day)` | `adopt_runs(ids::BOX, Uuid::now_v7(), TimeDelta::days(1))`. |
| `a_live_stranger` `:10204-10210` | `take_lease(run, BOX, owner, clock.now(), clock.now() + 1 day)` | `take_lease(run, ids::BOX, harness.orch.owner(), TimeDelta::days(1))`. |
| `Outlasting` verifier `:11170-11185` | `advance(ttl + 1 s)`; `take_lease(run, BOX, stranger, now, now + 1 day)` | `advance` unchanged (it also lapses the lease on the store's clock, the same `TestClock`); `take_lease(self.run, ids::BOX, self.stranger, TimeDelta::days(1))`; `let now` deleted if unused. |

Engine wrapper calls (`engine.take_lease(run)`, `engine.release_lease(run)`, `:8444-8536`,
`:11888`, `:11892`, `:12078`) are unchanged. `resume_on_another_box_s_run_with_a_lapsed_lease_is_not_lease_held`
(`:10037`) is unchanged and still green under B26 (another box → `RunStatus`). New:
`a_skewed_engine_still_names_a_live_lease_held` (§4.3).

#### 5.6 `htui-orch/src/recover.rs` tests (B28)

| Site | Old | New |
|---|---|---|
| `scripted` `:521-538` | records `(clock.now(), until)`; `impl FnMut(DateTime<Utc>) -> …`; `Vec<(DateTime, DateTime)>` | records `(clock.now(), ttl)`; `impl FnMut(TimeDelta) -> …`; `Vec<(DateTime<Utc>, TimeDelta)>`; doc "recording each `until`" → "recording each TTL". |
| `beats` `:549` | `&Mutex<Vec<(DateTime<Utc>, DateTime<Utc>)>>` | `&Mutex<Vec<(DateTime<Utc>, TimeDelta)>>`. |
| `heartbeat_refreshes_every_interval` `:577-580` | `for (beat, (now, until))`; `*until == *now + times.ttl` | `for (beat, (now, ttl))`; `assert_eq!(*ttl, times.ttl, "beat {beat}: every refresh asks for the configured TTL")`. The beat-time assertion is unchanged. |
| the hanging-refresh test `:703` | `\|_until\|` | `\|_ttl\|` (its type annotation, if any, `TimeDelta`). |

Every fence test (`heartbeat_fences_from_the_until_its_caller_wrote` `:654`,
`heartbeat_a_successful_refresh_moves_the_fence` `:720`, …) is unchanged: the fence is still
`written` and `clock.now() + ttl` on the local clock. Its doc "writes 170 s" stays true of the
local fence.

#### 5.7 The rest

| File:line | Old | New |
|---|---|---|
| `htui-orch/tests/review_loop.rs:83-90` | `claim_run(run.id, BOX, owner, now, now + TimeDelta::hours(1))` | `(run.id, ids::BOX, owner, now, chrono::TimeDelta::hours(1))`. |
| `htui-agent/tests/recorder.rs:3473-3479` | `claim_run(RUN_2, BOX, owner, at(), at() + 5 min)` | `(ids::RUN_2, ids::BOX, owner, at(), TimeDelta::minutes(5))`. |
| `htui-agent/tests/recorder.rs:3538-3551` | step (3): `take_lease(RUN_2, BOX, stranger, at() + 6 min, at() + 20 min)` "the stranger takes the lapsed lease" | **(E)** first `assert!(store.inner.refresh_lease(ids::RUN_2, owner, TimeDelta::zero()).await.expect("the refresh must not fail"), "the walk's lease lapses")`, then `take_lease(ids::RUN_2, ids::BOX, stranger, TimeDelta::minutes(14))`, same message. The refresh does not touch the step fence (`StepFence::Lease(owner)` compares `lease_owner`, which a refresh keeps), so the fence still admits `owner` until the take. |
| `htui/src/run_worker.rs:3495-3501` (`stranded`) | `claim_run(run, BOX, Uuid::now_v7(), past, past + 1 min)` | **(E)** `(run, ids::BOX, Uuid::now_v7(), past, TimeDelta::zero())`. Doc `:3459-3460` "claimed an hour ago and never renewed: `running`, its lease expired" → "claimed an hour ago (`started_at`) under a lease that lapsed at once: `running`, its lease expired". `:3024`, `:3572`, `:4043` unchanged (F-41). |
| `htui-orch/tests/gix_isolator.rs` | — | no lease call; B23's changes only (§2.5). |

---
### 6. Parallel implementer groups (disjoint files)

**T7** is one implementer (the files are few and `mem.rs` gates everything); §2's order.

**T8**, after T7 is green:

| Group | Files | Needs | Content |
|---|---|---|---|
| **G0** (serial, first) | `htui-core/src/store/{traits.rs, mod.rs, mem.rs (impl only, not mod tests), conformance.rs}`, `htui-store/src/{pg/write.rs, writer.rs}`, `htui-agent/src/conformance.rs`, `htui-agent/tests/recorder.rs` (the `SpyStore` forwards `:767-813` only), `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs` | — | §3.1-§3.4 (commit (1) shapes), `assert_leased_for`, §4.1, §5.1, the two count pins. Ends when `cargo build --workspace --all-features` passes **with the non-G0 test files not yet compiling** (`--lib` / `-p htui-core` tests run). |
| **G1** | `htui-core/src/store/mem.rs` (`mod tests` only, `:6239-`), `htui-store/tests/pg_criteria.rs` | G0 | §5.2, §5.3, §4.2. |
| **G2** | `htui-orch/src/{engine.rs, recover.rs}` | G0 | §3.5 (production), §5.5, §5.6, §4.3. |
| **G3** | `htui-orch/src/conformance.rs`, `htui-orch/tests/review_loop.rs` | G0 | §5.4, §5.7 row 1. |
| **G4** | `htui-agent/tests/recorder.rs` (`:3473`, `:3538-3551` only), `htui/src/run_worker.rs` | G0 | §5.7 rows 2-4. |

G1 and G0 both touch `mem.rs`, in disjoint regions (impl `:1-6238`, tests `:6239-`); G1 starts
after G0 has landed, so there is no concurrent edit. G4 and G0 both touch `recorder.rs` the same
way. G2's engine edits and G3's orch conformance never cross (`engine.rs` holds its own test
module). Then one serial commit (2): the gate in both stores' method bodies is already there
from G0 if G0 writes the final bodies; the §3.2 SQL, `cargo sqlx prepare`, B26's body, docs.

Simplest schedule that respects "each commit compiles": G0 writes **final** trait, Mem and
`Writer`/double code (the gate included) and the **commit-(1)** Pg bodies; G1-G4 in parallel;
commit (1) = all of it (red: `pg_criteria.rs::lease_times_are_the_databases`,
`a_skewed_engine_still_names_a_live_lease_held`, and `a_lease_ttl_out_of_range_is_refused` on
Pg only); commit (2) = §3.2 SQL + `.sqlx` + B26.

### 7. `.sqlx` accounting

| Point | Count | Change |
|---|---|---|
| `cd49a3b` | 281 | — |
| after M2-M3 (m2-m3 blueprint §8) | 288 | — |
| T7 | 288 | no SQL |
| T8 commit (1) | 288 | Pg bodies keep their old statements |
| T8 commit (2) | **288** | five queries change text, so five files are replaced: the admitted `claim_run` `UPDATE` (`query-108d76eb…`), `adopt_runs` (`query-27bb5e31…`), `release_lease` (`query-88c8e8d8…`), `refresh_lease` (`query-a2382e2f…`), `take_lease` (`query-b8e26eb2…`); five new hashes are added. Net 0. The `MissingTags` `UPDATE` and every `SELECT 1 FROM run WHERE id = $1` follow-up are byte-identical and keep their files. |

`cargo sqlx prepare` removes the five stale files itself; `git status` must show exactly five
`D` and five `A` under `.sqlx` in commit (2). No migration (0008 stays free).

### 8. File-by-file checklist

| File | T7 | T8 |
|---|---|---|
| `htui-core/src/clock.rs` (new) | §2.1 | — |
| `htui-core/src/lib.rs` | `pub mod clock;` | — |
| `htui-core/src/store/mem.rs` | `MemClock`, `with_clock`, `handle_at`, `now`; 61 + 2 stamps → `self.now()`; `a_mem_store_reads_its_clock` | §3.3; §5.2 |
| `htui-core/src/store/traits.rs` | — | §3.1 |
| `htui-core/src/store/mod.rs` | — | export `MAX_LEASE_TTL`, `lease_ttl_micros` |
| `htui-core/src/store/conformance.rs` | — | `LEASE`, `assert_leased_for`, §4.1, §5.1 |
| `htui-core/tests/mem_store.rs` | — | `:37` 95 → 96 |
| `htui-store/src/pg/write.rs` | — | §3.2 |
| `htui-store/src/writer.rs` | — | §3.4 |
| `htui-store/.sqlx/` | — | §7 |
| `htui-store/tests/pg_conformance.rs` | — | `:19` 95 → 96 |
| `htui-store/tests/pg_criteria.rs` | — | §5.3, §4.2 |
| `htui-agent/src/conformance.rs` | `epoch()` delegates (§2.2) | §3.4 |
| `htui-agent/tests/recorder.rs` | — | §3.4, §5.7 |
| `htui-orch/src/isolate.rs` | `:246-290` → re-export; the moved test goes | — |
| `htui-orch/src/fake.rs` | `:782-841` → re-export; `demo`/`restarted` clock their handle; the moved test goes | — |
| `htui-orch/src/recover.rs` | — | §3.5, §5.6 |
| `htui-orch/src/engine.rs` | — | §3.5, §5.5, §4.3 |
| `htui-orch/src/conformance.rs` | — | §5.4 |
| `htui-orch/tests/gix_isolator.rs` | §2.5 (`second_process`, `engine_as!`) | — |
| `htui-orch/tests/review_loop.rs` | — | §5.7 |
| `htui/src/run_worker.rs` | — | §5.7 (`stranded`) |
| `htui-core/Cargo.toml` | none (`chrono` is already a dependency; `test-support` exists) | — |

### 9. Gates

Environment and tooling as the m2-m3 blueprint's §10 (`source /home/user/htui-env.sh`,
`sqlx-cli 0.9.0`, `htui_sqlx` at `0007`). Every task ends with:

```bash
cargo fmt --all -- --check
cargo build --workspace --all-features --all-targets
cargo clippy --workspace --all-features --all-targets -- -D warnings
(cd crates/htui-store && DATABASE_URL=postgres://htui:htui@localhost:5432/htui_sqlx \
  cargo sqlx prepare --check -- --all-targets --all-features)
```

**T7:**

```bash
awk '/^impl WriteStore for MemStore/,/^#\[cfg\(all\(test/' crates/htui-core/src/store/mem.rs \
  | grep -c 'Utc::now()'                                         # 0
grep -rn 'struct TestClock\|struct SystemClock\|trait Clock' crates/*/src   # only htui-core/src/clock.rs
cargo test -p htui-core  --all-features -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
cargo test -p htui-orch  --all-features -- --test-threads=2
cargo build -p htui-core --no-default-features                   # clock.rs's gate holds
```

**T8** (after `cargo sqlx prepare` in commit (2); `ls crates/htui-store/.sqlx | wc -l` → 288):

```bash
grep -rn 'lease_until\|, now, now + \|release_lease([^)]*, [^)]*, ' crates --include=*.rs   # none
cargo test -p htui-core  --all-features -- --test-threads=2
cargo test -p htui-store --all-features --test pg_conformance --test pg_criteria -- --test-threads=2
cargo test -p htui-agent --all-features -- --test-threads=2
cargo test -p htui-orch  --all-features -- --test-threads=2
cargo test -p htui --all-features --lib -- --test-threads=2
cargo test -p htui-store --all-features --test pg_criteria lease_times_are_the_databases
cargo test -p htui-orch  --all-features a_skewed_engine_still_names_a_live_lease_held
cargo test -p htui-core  --all-features a_lease_ttl_out_of_range_is_refused
```

The P-11 flake check for B21 (run once, T7): `for i in $(seq 20); do cargo test -p htui-core
--all-features --test mem_store -q || break; done`.

### 10. PR #13 (MOD-58 claim-time tests)

Merged into `origin/main` (`ca2fb1f`); its `mem.rs` and `pg_criteria.rs` at `main` equal the
`mod-58` branch head. It adds four lease calls in each file, all `claim_run`/`refresh_lease` in
two tests per file; their rewrites are in §5.2 and §5.3 (the `@main` rows). It touches neither
`traits.rs` nor the Pg or Mem lease bodies, so §3 is unaffected. The branch merges `main`
before T8's G1 starts, or G1 applies the `@main` rows after the merge; either way the rewrite
is the table's.

### 11. `main` since `edf19c0`

`git diff edf19c0...origin/main -- crates/` (27 files): the T7/T8 files `conformance.rs`
(both), `traits.rs`, `pg/write.rs`, `writer.rs`, `recover.rs`, `run_worker.rs`,
`recorder.rs`, `review_loop.rs`, `isolate.rs` are untouched by `main`. `engine.rs` has three
disjoint hunks (around `@main` `:3133`, `:3718`, `:4539`; +17 lines net before the test
module), so every `engine.rs` test line in §5.5 shifts by +17 after the merge while every
production line in §3.5 is unchanged. `gix_isolator.rs` has one disjoint hunk (`@main`
`:3333`), below every §2.5 site. `mem.rs` and `pg_criteria.rs` carry PR #13 (§10).
`agent_worker.rs` changed heavily but holds no T7/T8 site. Migration `0008` is free and unused
here.
