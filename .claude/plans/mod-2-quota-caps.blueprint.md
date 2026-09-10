# Blueprint: MOD-2 milestone 6 close-out + milestone 7 — live `agy` turn, quota and caps

**Plan**: `.claude/plans/mod-2-quota-caps.plan.md` (D65–D73, T34 + T37–T44 binding; its "Verified
claims" table taken as read and re-checked against the tree on `main` at 2026-09-10 — every row
held except the three marked **plan ≠ tree** below).
**Design authority**: `docs/ANA-4.md` §7 (`:1077-1150`: the transport table, the payload
reconciliation, the `agent_box.quota` shape at `:1112-1125`, the passive latch `:1131-1135`,
`R-AGT-8`'s four skip rules `:1137-1141`, the enforcement point `:1143-1150` as amended by plan
D69), §11 criteria 7 and 8 (`:1356-1359`), §11.14 (`:1384-1390`). Where the plan and the tree
disagree the tree wins and the point is marked **plan ≠ tree**. Anything not verified in source is
marked **UNVERIFIED — implementer must check**.

Conventions inherited unchanged from the milestone-4/5/6 blueprints: `#![warn(missing_docs)]`,
`unsafe_code = "forbid"`, MSRV 1.98, `clippy::all` with `-D warnings`, one `thiserror` type per
crate (no new `DriverError` / `RecordError` / `StoreError` variant), **no new `DriverEvent` variant**
(D62/D66), no lock across an `.await`, `htui-store` never depends on `htui-agent` (build **or**
dev — `crates/htui-agent/Cargo.toml` dev-deps are `tokio`, `tempfile`, `insta` only, and `htui` is
the one crate holding both, `crates/htui/Cargo.toml:41-44`), the scrubber stays inside `Recorder`,
nothing keyed on `agent.name` (`R-AGT-5`: the quota source is `agent.settings.quota.source`,
`launch.rs:379-398`), caps are **USD micros** (D70), and the batch cap is MOD-12's (D71,
ANA-4:1283). No migration: `.sqlx/` regenerates for the two new `query!`s (README `:382-401`), and
MOD-4's `0003` stays free.

---

## Plan ≠ tree, resolved in the tree's favour

| # | Plan says | Tree says | Resolution |
|---|---|---|---|
| P-1 | D69: "`pump` — which holds `&mut dyn AgentSession` and the recorder together (`record.rs:965-982`) — calls `cancel(grace)`" | **Production never calls `pump`.** `run_chat` drives `run_turn` (`agent_worker.rs:2189-2200`, `:2300-2434`), whose own doc says it is "`pump`'s shape with one difference … the pull and the command channel have to be served by the same loop" (`:2290-2295`). `pump` is what the conformance suite (16 calls) and `tests/recorder.rs` (2) drive | The detection stays in `Recorder::record` and the cancel-and-close sequence becomes **one shared function**, `record::enforce_breach(session, recorder, breach)` (B.5), called from `pump` **and** from `run_turn`. `pump`'s signature is unchanged (the grace the cancel uses rides on the cap config, `RunCap.grace`, so 18 call sites do not move). The plan's "pump performs the cancel" is true for the suite and incomplete for the binary; both paths are tested (F-T40) |
| P-2 | File table: "`crates/htui-core/src/store/conformance.rs` — T38/T40: two new `CASES` entries and their scripts (13 → 15)" | That file is the **store** suite and has **20** cases (`store/conformance.rs:24-45`; `pg_conformance.rs:19` pins `EXPECTED_CASES = 20`). The 13-entry list is `crates/htui-agent/src/conformance.rs:127-141` (`acp_conformance.rs:85` and `fake_conformance.rs:30` pin 13) | Three cases, two suites. **Store** suite: `set_agent_box_quota_updates_two_columns_or_not_found` (T38, 20 → 21; `EXPECTED_CASES` → 21). **Agent** suite: `quota_blob_latches_agent_box` (T39) and `run_cap_breach_cancels_within_one_event` (T40), 13 → 15; both count guards → 15. The store suite is generic over `WriteStore` and has **no `agent_box` read**, so "probe byte-identical" is asserted where a read exists: `mem.rs` in-module (`MemStore::agents`) and `pg_criteria.rs` (raw `SELECT`) |
| P-3 | T43: criterion 7's ACP half "against live Postgres" in `crates/htui-store/tests/pg_criteria.rs` | `htui-store` cannot see the ACP mapper or a recorder (dependency direction; `chat_offline.rs:1-14`'s precedent: "only `htui` depends on both crates") | **New** `crates/htui/tests/chat_usage_pg.rs`: the recorded `claude_acp_turn.jsonl` through `Mapper` → `Recorder<Writer::Online>` over `htui_store::testkit::demo_db()`, the `replay.rs:392-417` shape. Skips with `testkit::SKIP` when the DSN is unset. The T39 "latch against live Postgres" lands in the same file; T38's setter-against-Postgres stays in `pg_criteria.rs` |
| P-4 | T40: the caps are read "at `ChatStart` … on the chat's own task" | `run_chat` holds a `Writer`, not the `Backend` (`ChatArgs`, `:1213-1232`); D70's `project_settings` is a `Backend` dispatch. `start()` already awaits `box_info`, `this_user` and `agents()` on the worker's task before the chat task exists (`:1072-1093`) | The read goes beside those three in `start()`; the result rides `ChatArgs` into `run_chat`. Off the UI task by the same argument as the three reads it joins (`R-NF-3`) |
| P-5 | T39: "`spend.session_micros` from the cumulative `cost_micros_total` the mapper already tracks" | The recorder already keeps the transport-neutral figure the cap compares against: `usage.cost_micros`, the sum of deltas (`record.rs:557`). For ACP it equals the last `cost_micros_total` — that is criterion 7's second clause, which T43 proves | `session_micros` **is** the recorder's running sum, so the latched figure and the cap verdict come from one number by construction. A `cli`/fake transport with deltas and no total gets a spend figure too |
| P-6 | D66: normalization "lives in a new `htui_core::model::quota`, selected by the row's declared `agent.settings.quota.source`" | `QuotaSource` is a `wire_enum!` in `htui-agent` (`launch.rs:384-398`); `htui-core` cannot name it | `QuotaSource` **moves** to `htui_core::model::quota` as a `str_enum!` (same four strings, `#[derive(Default)]` + `#[default] None` pass through the macro's `$(#[$enum_meta])*`); `launch.rs` gains `pub use htui_core::model::QuotaSource;` so `htui_agent::launch::QuotaSource` (`lib.rs:137`, `tests/launch.rs:20,341,354`) keeps resolving. `str_enum!` adds `FromStr` and, under `sqlx`, a `Type` derive — both harmless |
| P-7 | Risk table: T34's edit turn "works inside a scratch directory created by the test … `SessionSpec.cwd` is the bound" | `start()` sets `cwd = std::env::current_dir()` (`agent_worker.rs:1081`); nothing on `ChatStart` carries a directory | `chat_live_agy.rs` calls `std::env::set_current_dir(scratch.path())` before `ChatStart` and restores it at the end (a **safe** function — only `set_var` is `unsafe` in edition 2024, so `forbid(unsafe_code)` is untouched). Process-wide, which is acceptable because the file is a one-test binary run by hand |
| P-8 | T41: "the column and the statement, snapshot-pinned" | Four `settings__agents_*.snap` files exist (`demo`, `empty`, `probed`, `unknown_row`) and every one renders the table header and the idle hint | All four are **re-accepted** (a column and a hint line move, nothing else); one new snapshot `agents_quota` pins the three cell shapes |
| P-9 | T37's file set: `event.rs`, `acp/map.rs`, `tests/acp_map.rs`, fixtures | `tests/snapshots/acp_map__turn.snap` and `tests/snapshots/replay__acp_turn.snap` are `Debug` snapshots of the mapped fixture, and fixture line 5 already carries the blob | Both snapshots are re-accepted in T37 (the `Usage` event gains `quota: Some(..)`). No fixture is added for T37: the real one is the fixture |
| P-10 | T39's latch case "transport-neutral" | The ACP conformance harness re-encodes a `DriverEvent::Usage` as a `usage_update` with `used`/`size`/`cost` only (`acp_conformance.rs:355-368`) | The encoder gains the inverse of the mapper's read: `_meta: { "_claude/rateLimit": quota }` when `usage.quota` is `Some`. **File-set addition** to T39 (`crates/htui-agent/tests/acp_conformance.rs`) |
| P-11 | Verified claim: "`StopReason::Cancelled` exists … `event.rs:77`, `:93`" | `:77` is `TerminalReason::Cancelled`; `StopReason::Cancelled` is `:93` alone | Cosmetic; noted so nobody cites `:77` |
| P-12 | T38 file set: store crates only | Every `impl WriteStore` must gain the new method or nothing compiles: `MemStore`, `PgStore`, `BufferedWriter`, `Writer`, `UsageSpy` (`htui-agent/src/conformance.rs:495-540`), `SpyStore` (`htui-agent/tests/recorder.rs`), and any test double `rg 'impl.*WriteStore for'` finds | T38 adds a **delegating** arm to the two spies (one line each); T39 extends `UsageSpy` to record the call. T37 ∥ T38 ∥ T41 still holds (T37 never touches those files) |

---

## A. Per-file change table

| # | File | Action | Task | What changes (and what must **not**) |
|---|---|---|---|---|
| 1 | `crates/htui/tests/chat_live_agy.rs` | CREATE | T34 | B.8. `#[ignore]`, production runtime, scratch dir, three turns, fixture writer |
| 2 | `crates/htui-agent/tests/fixtures/agy_acp_turn.jsonl` | CREATE | T34 | The raw `session/update` lines the turn produced, same line shape as `claude_acp_turn.jsonl` (`{"direction","method","params"}`), redacted by hand (H-13) |
| 3 | `crates/htui-agent/tests/acp_map.rs` + `tests/snapshots/acp_map__agy_turn.snap` | UPDATE | T34 | `a_recorded_agy_turn_maps_to_the_rows_it_did_when_it_was_captured` over file 2 (milestone-6 blueprint A.19); a mapper edit **only** under D62's rule |
| 4 | `crates/htui-core/seeds/agent_agy.json` | UPDATE | T34 | D64 fields only if the turn/`session/new` supplied them; `settings.quota.source` stays `"none"` unless the wire showed a blob (then it is a **finding**, H-15). `crates/htui-core/src/model/agent.rs:212` asserts `"none"` and moves with it |
| 5 | `crates/htui-agent/src/event.rs` | UPDATE | T37 | `UsageEvent.quota: Option<Value>` after `usage_scope` (`:361`), B.2 |
| 6 | `crates/htui-agent/src/acp/map.rs` | UPDATE | T37 | `RATE_LIMIT_META_KEY`; `Mapper::usage` fills `quota` (`:74-107`); the module doc's "one field of carried state" sentence stays true (the blob is not state) |
| 7 | `crates/htui-agent/tests/acp_map.rs`, `tests/snapshots/{acp_map__turn,replay__acp_turn}.snap` | UPDATE | T37 | One new test (F-T37); two snapshots re-accepted (P-9) |
| 8 | `crates/htui-core/src/store/traits.rs` | UPDATE | T38 | `WriteStore::set_agent_box_quota` after `upsert_agent_box` (`:117`), B.3; the module doc `:8-14` ("adds six `WriteStore` methods … milestones 2 to 8 do not reopen this file") gains one sentence naming milestone 7's seventh |
| 9 | `crates/htui-core/src/store/mem.rs` | UPDATE | T38, T40 | T38: `State::set_agent_box_quota` after `upsert_agent_box` (`:850`), the async arm after `:1041`, an in-module test. T40: inherent `project_settings` after `agents()` (`:225`), B.4 |
| 10 | `crates/htui-core/src/store/conformance.rs` | UPDATE | T38, T45 | T38: `CASES` 20 → 21, `run_case` arm, the case after `upsert_agent_box_by_pk` (`:1260-1308`). T45: 21 → 22, `upsert_agent_box_cannot_write_quota` (B.11) |
| 10a | `crates/htui-agent/src/probe.rs` | UPDATE | T45 | D74: `agent_box_row` (`:1508-1530`) returns `quota: None, quota_at: None`; the doc paragraph at `:1500-1506` rewritten, incl. the corrected MOD-7 sentence; `existing` dropped if `version`'s fallback no longer reads it |
| 10b | `crates/htui-core/src/model/agent.rs` | UPDATE | T45 | D74: `AgentBox.quota` / `.quota_at` field docs (`:78-81`) name the single writer |
| 10c | `crates/htui-agent/tests/probe.rs` | UPDATE | T45 | `agent_box_row_does_not_carry_quota_forward` |
| 11 | `crates/htui-store/src/pg/write.rs` | UPDATE | T38 | The two-column `UPDATE` after `upsert_agent_box` (`:444`), B.3 |
| 12 | `crates/htui-store/src/writer.rs` | UPDATE | T38 | `BufferedWriter` arm → `registry_writes_need_the_server()` (`:273` neighbour); `Writer` arm after `upsert_agent_box` (`:459-465`); `REGISTRY_ON_SERVER_ONLY`'s doc (`:328-334`) gains "and the quota latch (D68)" |
| 13 | `crates/htui-store/tests/pg_conformance.rs` | UPDATE | T38, T45 | `EXPECTED_CASES` (`:19`) 20 → 21 → **22** |
| 14 | `crates/htui-store/tests/pg_criteria.rs` | UPDATE | T38, T45 | T38: `set_agent_box_quota_leaves_probe_byte_identical` (F-T38), raw `SELECT` like `step_usage` (`:98-105`). T45: the SQL-level proof that an upsert can neither set nor clear the two columns |
| 15 | `crates/htui-store/.sqlx/` | UPDATE | T38, T40 | One query file per new `query!` / `query_scalar!` (rows 11 and 17) |
| 16 | `crates/htui-agent/src/conformance.rs` (`UsageSpy`), `crates/htui-agent/tests/recorder.rs` (`SpyStore`) | UPDATE | T38 | Delegating `set_agent_box_quota` arms (P-12) |
| 17 | `crates/htui-store/src/pg/read.rs`, `src/cache/read.rs`, `src/backend.rs` | UPDATE | T40 | `project_settings` after each `agents()` (`pg/read.rs:605`, `cache/read.rs:760`, `backend.rs:298`), B.4 |
| 18 | `crates/htui-core/src/model/quota.rs` | CREATE | T39, T42 | T39: `QuotaSource`, `Quota`, `QuotaWindow`, `Spend`, `normalize`, `to_value`, `from_value`, `tightest_window`, `ProjectCaps`, `CapError`. T42: `Availability`, `SkipReason`, `available` (B.1) |
| 19 | `crates/htui-core/src/model/mod.rs` | UPDATE | T39, T42 | `pub mod quota;` between `note` and `run` (`:89-90`); re-exports after `pub use note::Note;` (`:112`) |
| 20 | `crates/htui-agent/src/launch.rs`, `src/lib.rs` | UPDATE | T39 | P-6: the `wire_enum!` at `:384-398` deleted, `pub use htui_core::model::QuotaSource;` in its place; `lib.rs:137` unchanged |
| 21 | `crates/htui-agent/src/record.rs` | UPDATE | T39, T40 | T39: `QuotaLatch`, `with_quota_latch`, `latch_quota`, `last_quota_raw`. T40: `RunCap`, `CapBreach`, `CAP_EXCEEDED`, `with_run_cap`, `check_cap`, `record` → `Result<Option<CapBreach>, _>`, `record_cap_breach`, `enforce_breach`, `pump`'s breach arm, `RecorderSummary.cap_breach` (B.5). Module doc item list gains a fifth item (the latch and the cap) |
| 22 | `crates/htui-agent/tests/recorder.rs` | UPDATE | T39, T40 | F-T39, F-T40 recorder-level cases; `SpyStore` records the setter and can be told to refuse |
| 23 | `crates/htui-agent/src/conformance.rs` | UPDATE | T39, T40 | `CASES` 13 → 15; `UsageSpy.quota_calls`; the two cases (B.9) |
| 24 | `crates/htui-agent/tests/acp_conformance.rs`, `tests/fake_conformance.rs` | UPDATE | T39 | Count guards 13 → 15 (`acp_conformance.rs:85`, `fake_conformance.rs:30`); the `_meta` encoder (P-10) |
| 25 | `crates/htui/src/agent_worker.rs` | UPDATE | T40 | `start()` reads the caps and builds the latch; `ChatArgs.caps` / `.quota_latch`; `run_chat` configures the recorder; `TurnEnd::CapExceeded`; `run_turn`'s breach arm; `quota_latch_for`; in-module tests (B.6, F-T40) |
| 26 | `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | T41 | `quota_cell`, the eighth column, widths, `HINT_IDLE` (B.7); the "seven-column table" doc at `:943` → eight |
| 27 | `crates/htui/tests/settings.rs`, `tests/snapshots/settings__agents_{demo,empty,probed,unknown_row,quota}.snap` | UPDATE / RE-ACCEPT / CREATE | T41 | F-T41 |
| 28 | `crates/htui/tests/chat_usage_pg.rs` | CREATE | T43 | P-3: criterion 7's ACP clause and the latch, against live Postgres |
| 29 | `README.md`, `HANDOFF.md`, `.claude/prds/mod-2-agent-driver-chat.prd.md:155-156`, this plan's answer table | UPDATE (docs) | T44 | README: the two `project.settings` keys and their unit in the Chat-tab section beside the env-knob table (`:227-232`), plus the Settings quota column and the `r` statement; HANDOFF phase-7 note on the MOD-2 line (`:77`); PRD rows 6 and 7 → `complete` |

No manifest changes: `chrono` already carries `serde` (workspace `Cargo.toml:16`), `htui-core` has
`serde_json`, `htui-agent` has `tracing`, and the `htui` dev-deps already hold `htui-store` with
`test-support` (`crates/htui/Cargo.toml:41-44`).

---

## B. Interfaces, exactly

### B.1 `crates/htui-core/src/model/quota.rs` (T39, T42)

Module doc, first paragraph: the §7 blob is the one document three readers share — the recorder
writes it, the Settings tab renders it, MOD-4's selection loop reads it — and `htui-core` holds it
for the same reason `usage.rs` holds `UsageTotals` (`usage.rs:8-12`): two crates that may not depend
on each other both need one definition. `to_value()` is hand-built and pinned to the serde form, as
`usage.rs:73-85` does and says why.

```rust
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use crate::model::Billing;

str_enum!(
    /// `agent.settings.quota.source` (`docs/ANA-4.md` §5.2, §7): where a row's allowance is read
    /// from. Declared on the row, never derived from its name (`R-AGT-5`). Lives here rather than
    /// in the driver crate since MOD-2 milestone 7 because `normalize` selects on it and this
    /// crate cannot see `htui-agent`; `htui_agent::launch` re-exports it unchanged.
    #[derive(Default)]
    QuotaSource {
        /// ACP's `_meta["_claude/rateLimit"]` on a `usage_update`.
        AcpMetaRateLimit => "acp_meta_rate_limit",
        /// A rate-limit event in the CLI's JSON stream (milestone 8).
        CliRateLimitEvent => "cli_rate_limit_event",
        /// The CLI's status line (milestone 8).
        CliStatusLine => "cli_status_line",
        /// No quota is reported. The default: a row that says nothing promises nothing.
        #[default]
        None => "none",
    }
);

/// One allowance window: `{ id, utilization, resets_at }` (§7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaWindow {
    /// The vendor's window id, e.g. `five_hour`, `seven_day`.
    pub id: String,
    /// The 0..1 fraction consumed; `>= 1.0` is a full window (§7's second skip rule).
    pub utilization: f64,
    /// The vendor's epoch converted to a timestamp; `None` when the window named none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
}

/// `quota.spend`: what this session has cost so far, in USD micros — the recorder's running
/// `cost_micros` sum, which for an ACP session equals the last `cost_micros_total` (§11 criterion 7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Spend {
    /// `None` when no USD cost has been reported this session (a context-only transport, or a
    /// non-USD one: a cap in micros has nothing to compare against either way, H-5).
    pub session_micros: Option<i64>,
    /// `"USD"` whenever `session_micros` is `Some`; micros are USD by definition (`UsageEvent`).
    pub currency: Option<String>,
}

/// The `agent_box.quota` document of `docs/ANA-4.md` §7, key order as declared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    pub source: QuotaSource,
    pub billing: Billing,
    /// The vendor's status string verbatim (`allowed`, `allowed_warning`, `rejected`, …); `None`
    /// when the source reports none. Only a present, non-`allowed` value is a skip (D72, H-7).
    #[serde(default)] pub status: Option<String>,
    /// Derived, never reported: `status == "rejected"` or any window at `utilization >= 1.0`.
    #[serde(default)] pub exhausted: bool,
    /// Sorted by `id`, so two latches of one blob are byte-identical.
    #[serde(default)] pub windows: Vec<QuotaWindow>,
    #[serde(default)] pub spend: Spend,
    /// `agent_box.quota_at` mirrors this (§7).
    pub observed_at: DateTime<Utc>,
}

impl Quota {
    /// The seven keys in declaration order, hand-built (infallible) and pinned to the derived
    /// form by `to_value_matches_the_serde_form`. Timestamps as chrono's serde writes them:
    /// `to_rfc3339_opts(SecondsFormat::AutoSi, true)`.
    #[must_use] pub fn to_value(&self) -> Value;
    /// A stored document back into the type; `None` for anything that does not parse (a
    /// pre-milestone-7 hand-written blob such as the fixture's `{ "remaining": 100 }`).
    #[must_use] pub fn from_value(value: &Value) -> Option<Self>;
    /// The window with the highest `utilization` (first by `id` on a tie): what Settings shows.
    #[must_use] pub fn tightest_window(&self) -> Option<&QuotaWindow>;
}

/// §7's document from what one `usage` row and its row-side facts say (plan D66, P-5).
///
/// `raw` is the vendor blob **as scrubbed and persisted** on the `usage` row — never the wire
/// message — so the latched document can carry nothing the row does not. For
/// [`QuotaSource::AcpMetaRateLimit`]: `status` = `raw.status` as a string; `windows` = every
/// entry of `raw.unifiedWindows` whose `utilization` is a number, `resets_at` from `resetsAt`
/// as epoch seconds through `DateTime::from_timestamp(secs, 0)` (`None` if absent or out of
/// range), sorted by id. Any other source, or `raw == None`: no status, no windows,
/// `exhausted: false`. `spend` is `session_micros` with `"USD"` iff `Some`.
#[must_use]
pub fn normalize(
    source: QuotaSource,
    billing: Billing,
    raw: Option<&Value>,
    session_micros: Option<i64>,
    observed_at: DateTime<Utc>,
) -> Quota;

/// `project.settings.per_token_cap_run` / `per_token_cap_batch` (plan D70, D71): USD micros.
pub const PER_TOKEN_CAP_RUN: &str = "per_token_cap_run";
pub const PER_TOKEN_CAP_BATCH: &str = "per_token_cap_batch";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectCaps {
    /// Enforced by the recorder per chat run (D70). `None` = unbounded.
    pub run_micros: Option<i64>,
    /// Read and reported, **not** enforced here: MOD-12's (D71, ANA-4:1283).
    pub batch_micros: Option<i64>,
}

/// A cap key that is present and not a non-negative integer. Plain struct with a hand-written
/// `Display` (as `model::ParseEnumError`): `htui-core`'s one `thiserror` type is `StoreError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapError { pub key: &'static str, pub found: String }
// Display: "project.settings.{key} must be a non-negative integer of USD micros, got {found}"

impl ProjectCaps {
    /// Absent or `null` → `None`; an integer `>= 0` → `Some`; anything else (negative, float,
    /// string, bool, object) → `Err`, because a cap the operator wrote and `htui` ignored is the
    /// risk table's "wrong by a factor of a million" in the other direction. `0` is a real cap:
    /// it cancels on the first row that reports any USD cost.
    pub fn from_settings(settings: &Value) -> Result<Self, CapError>;
}

/// `R-AGT-8`'s verdict (plan D72). MOD-4 calls it; MOD-2 tests it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability { Available, Skip(SkipReason) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    Exhausted,
    WindowFull { id: String },
    Status(String),
    CapReached { spent_micros: i64, cap_micros: i64 },
}

/// §7's four skip rules in §7's order, over the stored blob: `exhausted`; any window
/// `utilization >= 1.0`; a present `status != "allowed"`; `spend >= cap` when both are `Some`.
/// A `None` quota, and a blob that does not parse, are **unknown and available** — otherwise a
/// row that reports nothing would never be selected (§7's own sentence about `agy`). `spend`
/// and `cap` are the caller's run- or batch-level figures, not the blob's session spend: a
/// session is not a run.
#[must_use]
pub fn available(quota: Option<&Value>, spend: Option<i64>, cap: Option<i64>) -> Availability;
```

`mod.rs`: `pub mod quota;` between `pub mod note;` and `pub mod run;`;
`pub use quota::{Availability, CapError, ProjectCaps, Quota, QuotaSource, QuotaWindow, SkipReason, Spend, available, normalize};`
after `pub use note::Note;`.

### B.2 `UsageEvent.quota` and the mapper (T37)

`event.rs`, after `usage_scope` (`:358-361`):

```rust
/// Added key (§7, plan D66): the transport's vendor rate-limit blob, **verbatim**. Over ACP it
/// is `_meta["_claude/rateLimit"]` of the `usage_update`, present on some reports and not on
/// others (`tests/acp_map.rs`: the first report of a turn carries none). Not one of the five
/// summed keys — `UsageTotals::add_payload` reads five fixed names (`usage.rs:48-54`) — and
/// normalized into `agent_box.quota` by `htui_core::model::quota::normalize`, never here.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub quota: Option<Value>,
```

`acp/map.rs`, beside `MICROS_PER_UNIT` (`:29`):

```rust
/// The one `_meta` key `docs/ANA-4.md` §7 documents on a `usage_update`. A wire key read for
/// **every** ACP agent, not an agent-name dispatch: an adapter that does not send it maps to
/// `quota: None`, which is what `R-AGT-5` requires and what `agy` is expected to do (§11.14).
pub const RATE_LIMIT_META_KEY: &str = "_claude/rateLimit";
```

`Mapper::usage` (`:74-107`), after the `match (amount, currency)` block and before `event`:

```rust
event.quota = update
    .get("_meta")
    .and_then(|meta| meta.get(RATE_LIMIT_META_KEY))
    .filter(|blob| blob.is_object())
    .cloned();
```

### B.3 `WriteStore::set_agent_box_quota` (T38, D67)

`traits.rs`, after `upsert_agent_box` (`:117`):

```rust
/// Writes `agent_box.quota` and `quota_at` of one **existing** row and nothing else — never
/// `probe`, `enabled`, `version` or `path` (MOD-2 plan D67; `docs/ANA-4.md` §7's passive
/// latch). Narrow on purpose: the latch runs inside a chat while a re-probe of the same row
/// may be running beside it (plan D55/D60), and two writers of one row must not be one
/// statement wide. No insert: a row that has never been probed has no columns to latch into.
///
/// # Errors
///
/// [`StoreError::NotFound`](crate::store::StoreError::NotFound) with `entity: "agent_box"` and
/// id `"<agent_id>/<box_id>"` when no row has that key.
async fn set_agent_box_quota(
    &self,
    agent_id: AgentId,
    box_id: BoxId,
    quota: Value,
    quota_at: DateTime<Utc>,
) -> Result<()>;
```

(`AgentId`, `BoxId` join the `use crate::model::{…}` list at `:19-23`.)

`mem.rs` — `State`, after `upsert_agent_box` (`:850`):

```rust
/// D67: two columns on an existing row, `updated_at` bumped as `set_step_usage` bumps it.
fn set_agent_box_quota(&mut self, agent_id: AgentId, box_id: BoxId, quota: Value, quota_at: DateTime<Utc>, now: DateTime<Utc>) -> Result<()> {
    let row = self.agent_boxes.get_mut(&(agent_id, box_id)).ok_or_else(|| StoreError::NotFound {
        entity: "agent_box", id: format!("{agent_id}/{box_id}"),
    })?;
    row.quota = Some(quota);
    row.quota_at = Some(quota_at);
    row.updated_at = now;
    Ok(())
}
```

and the `impl WriteStore for MemStore` arm after `:1041`:
`let now = Utc::now(); self.write(|state| state.set_agent_box_quota(agent_id, box_id, quota, quota_at, now))`.

`pg/write.rs`, after `upsert_agent_box` (`:444`):

```rust
/// D67: two columns, keyed on the composite primary key; `updated_at` is the migration's
/// `BEFORE UPDATE` trigger's. `rows_affected() == 0` is the `NotFound`.
async fn set_agent_box_quota(&self, agent_id: AgentId, box_id: BoxId, quota: Value, quota_at: DateTime<Utc>) -> Result<()> {
    let updated = sqlx::query!(
        "UPDATE agent_box SET quota = $3, quota_at = $4 WHERE agent_id = $1 AND box_id = $2",
        agent_id.as_uuid(), box_id.as_uuid(), &quota, quota_at,
    )
    .execute(&self.pool).await.map_err(map_sqlx)?.rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound { entity: "agent_box", id: format!("{agent_id}/{box_id}") });
    }
    Ok(())
}
```

**UNVERIFIED — implementer must check** that `agent_box` is in the trigger loop at
`migrations/0001_init.sql:570-580` (every table with `updated_at` should be; the loop iterates a
list). If it is not, add `, updated_at = now()` to the `SET` list so the two backends agree with the
`mem.rs` arm.

`writer.rs`: `BufferedWriter` → `Err(registry_writes_need_the_server())` (beside `:273-275`);
`Writer` → the three-arm delegation after `:465`. `REGISTRY_ON_SERVER_ONLY` is the sentence D68
wants the worker to log (B.6).

Store conformance case (`store/conformance.rs`, after `upsert_agent_box_by_pk`), name
`set_agent_box_quota_updates_two_columns_or_not_found`: upsert the `:1261-1272` row;
`set_agent_box_quota(ids::AGENT_CLAUDE, ids::BOX, json!({"source":"none","spend":{"session_micros":7,"currency":"USD"}}), Utc::now())`
→ `Ok`; the same call with `AgentId::new()` → `Err(StoreError::NotFound { entity: "agent_box", .. })`.
No read-back here (the suite has no `agent_box` read); the byte-identical proofs are F-T38.

### B.4 `project_settings(ProjectId)` — the `agents()` shape (T40, D70)

```rust
// htui-core/src/store/mem.rs, after `agents()` (:225)
/// `project.settings` of one project, or `None` when the store holds no such project. Inherent
/// for the reason `agents()` is (`Backend` dispatches, no trait method): the caps are read once
/// at `ChatStart` (MOD-2 plan D70) and the mirror carries the column, so every backend answers.
pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
    Ok(self.read(|state| state.projects.get(&project).map(|row| row.settings.clone())))
}

// htui-store/src/pg/read.rs, after `agents()` (:605)
pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
    sqlx::query_scalar!("SELECT settings FROM project WHERE id = $1", project.as_uuid())
        .fetch_optional(&self.pool).await.map_err(map_sqlx)
}

// htui-store/src/cache/read.rs, after `agents()` (:760) — runtime query, no `.sqlx` entry
pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
    let row = sqlx::query("SELECT settings FROM project WHERE id = ?")
        .bind(project.to_string()).fetch_optional(&self.pool).await.map_err(map_sqlx)?;
    row.map(|row| json_col("project.settings", &text(&row, "settings")?)).transpose()
}

// htui-store/src/backend.rs, after `agents()` (:298)
pub async fn project_settings(&self, project: ProjectId) -> Result<Option<Value>> {
    match self {
        Self::Memory(store) => store.project_settings(project).await,
        Self::Online { pg, .. } => pg.project_settings(project).await,
        Self::Offline { cache, .. } => cache.project_settings(project).await,
    }
}
```

`settings` is `JSONB NOT NULL` (`0001_init.sql:150`), so `query_scalar!` types it
`serde_json::Value` and `fetch_optional` gives the `Option`. The mirror column is
`TEXT NOT NULL DEFAULT '{}'` (`0001_mirror.sql:57-61`).

### B.5 `crates/htui-agent/src/record.rs` (T39, T40, D69)

New imports: `std::time::Duration`; `htui_core::model::{AgentId, Billing, BoxId, QuotaSource, quota::normalize}`;
`crate::event::{ErrorEvent, StopReason}`.

Constants and types, after `SCRUB_RESIDUE` (`:80`):

```rust
/// `error.code` of the row the recorder writes when the per-run cap is reached (§7, criterion 8).
pub const CAP_EXCEEDED: &str = "cap_exceeded";

/// What the passive latch of `docs/ANA-4.md` §7 needs (plan D66–D68): which row to write, and the
/// two row-side facts the document carries. `source` is `agent.settings.quota.source` and
/// `billing` is `agent.billing` — never the agent's name (`R-AGT-5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaLatch { pub agent_id: AgentId, pub box_id: BoxId, pub source: QuotaSource, pub billing: Billing }

/// The per-run cap (plan D70) and the grace the cancel it triggers may take (plan D69). The
/// grace rides here so [`pump`] keeps its signature: the worker passes its `CANCEL_GRACE`, the
/// conformance suite zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunCap { pub micros: i64, pub grace: Duration }

/// The verdict [`Recorder::record`] returns **once** per session: the row that took the running
/// USD spend to or past the cap (`spent >= cap`, the same `>=` as `R-AGT-8`'s "already reached").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapBreach {
    pub cap_micros: i64,
    pub spent_micros: i64,
    /// The breaching row's capture time.
    pub at: DateTime<Utc>,
}
```

`RecorderSummary` gains `pub cap_breach: Option<CapBreach>` (still `Eq`). `Recorder` gains, after
`usage_dirty`:

```rust
/// D66–D68: `None` records no quota (a test, an offline chat, a backend that refused once).
quota_latch: Option<QuotaLatch>,
/// The last vendor blob a `usage` row of this session carried, scrubbed. Kept so a later row
/// without one refreshes spend without erasing the windows (H-3).
last_quota_raw: Option<Value>,
run_cap: Option<RunCap>,
/// Set by the first breach; a later row never reports a second.
cap_breached: Option<CapBreach>,
```

`Debug` prints `run_cap`, `cap_breached` and `quota_latch.is_some()`. Builders after `new` (`:299`),
no change to `new`'s five parameters:

```rust
#[must_use] pub fn with_quota_latch(mut self, latch: QuotaLatch) -> Self;
#[must_use] pub fn with_run_cap(mut self, cap: RunCap) -> Self;
#[must_use] pub const fn run_cap(&self) -> Option<RunCap>;
```

`record` becomes `pub async fn record(&mut self, envelope: DriverEnvelope) -> Result<Option<CapBreach>, RecordError>`.
The two early returns become `.map(|()| None)` on `refuse`, and `record_unreadable` keeps its own
return (below). The `(event, _)` arm (`:551-569`):

```rust
(event, _) => {
    let mut breach = None;
    if matches!(event, DriverEvent::Usage(_)) {
        self.usage.add_payload(&payload);          // unchanged comment
        self.usage_dirty = true;
        self.latch_quota(&payload, scrubbed.at).await;
        breach = self.check_cap(scrubbed.at);
    }
    self.push(PendingRow { /* unchanged */ });
    self.buffer.len() - 1
}
```

(`breach` is declared before the `match` and returned as `Ok(breach)` after `send_ui`.)
`record_unreadable` (`:720-745`) returns `Result<Option<CapBreach>, RecordError>`, adds
`let breach = self.check_cap(at);` right after its `add_payload`, and **does not latch** — a masked
document is not a document to publish. New private methods after `sync_step` (`:694`):

```rust
/// The passive latch (§7): runs after every `usage` row, writes when the row has something to
/// say, and never fails the turn (plan D68).
async fn latch_quota(&mut self, payload: &Value, at: DateTime<Utc>) {
    let Some(latch) = self.quota_latch else { return };
    if let Some(raw) = payload.get("quota").filter(|raw| raw.is_object()) {
        self.last_quota_raw = Some(raw.clone());
    }
    // Nothing to latch yet: no blob this session and no spend. The row that stands keeps the
    // last session's windows rather than being erased by a bare first report (H-3).
    if self.last_quota_raw.is_none() && self.usage.cost_micros.is_none() { return; }
    let quota = normalize(latch.source, latch.billing, self.last_quota_raw.as_ref(), self.usage.cost_micros, at);
    match self.store.set_agent_box_quota(latch.agent_id, latch.box_id, quota.to_value(), at).await {
        Ok(()) => {}
        Err(StoreError::Unreachable(reason)) => {
            tracing::info!(%reason, "quota is not latched on this backend; the usage rows are (plan D68)");
            self.quota_latch = None;
        }
        Err(StoreError::NotFound { .. }) => {
            tracing::debug!("no agent_box row for this agent on this box; nothing to latch into until it is probed");
            self.quota_latch = None;
        }
        Err(err) => tracing::warn!(%err, "the quota latch failed; the turn continues (plan D68)"),
    }
}

/// The cap comparison (§7, plan D70): `Some` exactly once, on the row that reaches the cap.
fn check_cap(&mut self, at: DateTime<Utc>) -> Option<CapBreach> {
    let cap = self.run_cap?;
    let spent = self.usage.cost_micros?;
    if self.cap_breached.is_some() || spent < cap.micros { return None; }
    let breach = CapBreach { cap_micros: cap.micros, spent_micros: spent, at };
    self.cap_breached = Some(breach);
    Some(breach)
}

/// The two closing rows of §7 and §11 criterion 8, in this order and last: `error {
/// code: "cap_exceeded", message }` with role `htui` (as every row the recorder authors), then
/// `done { stop_reason: "cancelled" }`. `transport_done` is the transport's own turn end, taken
/// off the stream by [`enforce_breach`] and **not** recorded as it stood: its `stop_reason` may be
/// `end_turn` when the agent finished inside the grace window, and criterion 8 says the last row
/// says `cancelled`. Its `raw` (scrubbed; dropped rather than refused on residue) and `at` are
/// kept on the synthesized row so `keep_raw_events` still explains it. Both rows go to the UI.
pub async fn record_cap_breach(&mut self, breach: CapBreach, transport_done: Option<DriverEnvelope>) -> Result<(), RecordError>;
```

Message text (says "estimated", risk table):
`format!("per-run cap reached: an estimated ${:.4} spent against a cap of ${:.4} (project.settings.per_token_cap_run = {} micros); the session was cancelled", spent/1e6, cap/1e6, cap)`.
Body order: `flush` → push error row at `breach.at` → push done row at `transport_done.at` or
`breach.at` → `flush` → `sync_step` → `send_ui` the two envelopes (`raw: None`). `finish` fills
`cap_breach: self.cap_breached`.

The shared cancel, after `pump` (`:982`):

```rust
/// D69's cancellation, performed by the layer that holds the session — `pump` here, `run_turn`
/// in the worker — because `Recorder` holds none and must not (it is what the conformance suite
/// drives with no transport). Order: `cancel(grace)`; pull what the cancel produced (synthesized
/// `tool_result`s, answers) into the log **up to** the transport's `done`, which is withheld;
/// then the two closing rows. Returns the `cancelled` end for the caller's turn loop.
///
/// # Errors
/// The recorder's, mapped through [`RecordError`]. A failed `cancel` is logged, not returned: the
/// kill path already ended the session and the rows still have to be written.
pub async fn enforce_breach<S: WriteStore>(
    session: &mut dyn AgentSession,
    recorder: &mut Recorder<'_, S>,
    breach: CapBreach,
) -> Result<DoneEvent, DriverError> {
    let grace = recorder.run_cap().map_or(Duration::ZERO, |cap| cap.grace);
    if let Err(err) = session.cancel(grace).await {
        tracing::warn!(%err, "the cap's cancel took the kill path");
    }
    let mut transport_done = None;
    while let Ok(Some(envelope)) = session.next_event().await {
        if matches!(envelope.event, DriverEvent::Done(_)) { transport_done = Some(envelope); break; }
        recorder.record(envelope).await?;   // the verdict is spent; `check_cap` answers None now
    }
    recorder.record_cap_breach(breach, transport_done).await?;
    Ok(DoneEvent { stop_reason: StopReason::Cancelled })
}
```

`pump` (`:965-982`):
`if let Some(breach) = recorder.record(envelope).await? { return enforce_breach(session, recorder, breach).await; }`
before the `if let Some(done)` return. Both transports queue their post-cancel rows and then a
`Done(Cancelled)` (`fake.rs:404-436`; `acp/mod.rs:688-720` + `cancel_session` `:1670-1712`), so the
loop terminates on the `done` or on the channel closing.

### B.6 `crates/htui/src/agent_worker.rs` (T40)

`start()` (`:1053-1209`), after the `settings` parse (`:1107-1108`):

```rust
// Plan D70: the caps are the project's, read once here beside the three registry reads above
// — the worker's task, not the UI's (`R-NF-3`) — and carried into the chat's own task.
let project_settings = backend.project_settings(project_id).await?.ok_or_else(|| StoreError::NotFound {
    entity: "project", id: project_id.to_string(),
})?;
let caps = ProjectCaps::from_settings(&project_settings)
    .map_err(|err| StoreError::Constraint(err.to_string()))?;   // → `Failed { request: "chat_start" }`
if caps.batch_micros.is_some() {
    tracing::info!(project = %project_id, "per_token_cap_batch is set; a chat does not enforce it (ANA-4 §9: MOD-12 does)");
}
let quota_latch = quota_latch_for(&writer, &summary.agent, box_id, settings.quota.source);
```

```rust
/// D66–D68: the latch a chat records with, or `None` with the reason logged. A buffered writer
/// refuses registry writes with `REGISTRY_ON_SERVER_ONLY` (`recording_writer`, above), so the
/// decision is made here rather than discovered on the first `usage` row: the usage rows are
/// buffered and re-derive the figure after upload; the last server-side value stands meanwhile.
fn quota_latch_for(writer: &Writer, agent: &Agent, box_id: BoxId, source: QuotaSource) -> Option<QuotaLatch> {
    if matches!(writer, Writer::Buffered(_)) {
        tracing::info!(agent = %agent.name, reason = htui_store::REGISTRY_ON_SERVER_ONLY, "offline: quota is not latched (plan D68)");
        return None;
    }
    Some(QuotaLatch { agent_id: agent.id, box_id, source, billing: agent.billing })
}
```

`ChatArgs` gains `caps: ProjectCaps` and `quota_latch: Option<QuotaLatch>` after `reprobe`; `Debug`
prints both. `run_chat` (`:2166-2172`):

```rust
let mut recorder = Recorder::new(&writer, &scrubber, step_id, retain_raw, Some(ui_tx));
if let Some(latch) = quota_latch { recorder = recorder.with_quota_latch(latch); }
if let Some(micros) = caps.run_micros { recorder = recorder.with_run_cap(RunCap { micros, grace }); }
```

`TurnEnd` (`:2091-2096`) gains
`/// The per-run cap was reached: the session is cancelled and the run fails (§7). CapExceeded`.
`run_chat`'s match (`:2201-2214`):
`Ok(TurnEnd::CapExceeded) => { status = RunStatus::Failed; last_stop = StopReason::Cancelled; break; }`
— the existing tail then runs `finish`, `close_run(Failed)` and `frames.ended(Cancelled)`; the error
row reached the tab through the recorder's channel (D73: "arrives as the error row the chat renders
today"). `run_turn` (`:2389-2396`):

```rust
if let Some(breach) = record(recorder, envelope, ui, frames).await {
    enforce_breach(session, recorder, breach).await?;
    forward(ui, frames);                                   // the two closing frames
    return Ok(TurnEnd::CapExceeded);
}
```

where the `record` helper (`:2440-2452`) returns `Option<CapBreach>` (`Ok(breach) => breach`, `Err`
→ log, `None`) and its `while let Ok(frame) = ui.try_recv()` loop is extracted as
`fn forward(ui, frames)`; `drain` (`:2455-2468`) discards the verdict (the session is already being
cancelled). `grace` stays a `run_turn` parameter for the user-cancel arm (`:2352`).

### B.7 `crates/htui/src/ui/tabs/settings/agents.rs` (T41, D73)

```rust
/// The `quota` column (`R-TUI-8`, plan D73), read off `agent_box.quota` the way `on_box_cell`
/// reads `probe.status`: by key, so this column needs nothing from the driver crate. The
/// tightest window as a percentage with its reset when the blob has windows; the session spend
/// when it has only that; `—` for no row, no blob, or a blob this build does not understand.
/// Refreshing is not offered: a probe handshake reports no allowance, so `r` cannot (§7), and
/// [`HINT_IDLE`] says so.
fn quota_cell(summary: &AgentSummary) -> String
```

Rules: `quota = summary.on_box.as_ref()?.quota.as_ref()?`; windows = `quota["windows"]` array
entries with a numeric `utilization`; tightest = max by `utilization` (first on tie) →
`format!("{:.0}% to {}", u * 100.0, resets)` with `resets` = `resets_at` parsed as RFC3339 through
`DateTime::parse_from_rfc3339` and rendered `%m-%d %H:%M` UTC, or `format!("{:.0}%", u * 100.0)`
when the window has none; else `quota["spend"]["session_micros"].as_i64()` →
`format!("${:.2} spent", micros as f64 / 1e6)` (the `usage_line` cast precedent,
`chat/transcript.rs:575`); else `NONE`. `{:.0}` keeps the percentage free of an integer cast.

Table (`:953-997`): the `quota` cell is the **seventh**, `on this box` stays eighth so the `Min`
column stays last; header gains `Cell::from("quota")`; widths gain `Constraint::Length(19)` before
`Min(11)` (fixed widths 55 + 19 + 11 + 7 gaps = 92 ≤ 100 — the re-accepted snapshots are the check,
H-12). `HINT_IDLE` (`:67`) becomes
`"j/k select · r probe · i install · a authenticate · quota latches per chat, r cannot refresh it"`.

### B.8 `crates/htui/tests/chat_live_agy.rs` (T34)

The milestone-6 blueprint G-T34 stands as written (probe first, then chat; `ChatAccepted.caps`
check; three turns; the assertion → §11.14 table;
`HTUI_KEEP_RAW_EVENTS=1 HTUI_AGY_FIXTURE_OUT=… cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture`).
Four amendments:

1. **Scratch directory** (P-7): `let scratch = tempfile::tempdir()?; let home = std::env::current_dir()?; std::env::set_current_dir(scratch.path())?;`
   before the probe, and `set_current_dir(home)` in a guard struct's `Drop` so a panic mid-turn still
   restores it. Turn 3 asks for `htui-agy-probe.txt` "in the working directory"; the tempdir removes
   it, so milestone-6 H-10's manual cleanup is gone.
2. **Permission-worthiness**: turn 3's file write is the request that `default` mode should gate. The
   test answers **every** `PermissionRequest` frame with the first option whose `kind == AllowOnce`
   via `StoreRequest::ChatAnswer { step_id, request_id, answer: PermissionAnswer::Selected(id) }`
   (the `chat_live.rs:83-89` request shape), printing `options[].{id, kind}` first — that print is
   §11.14's answer. A turn that produces the file with **no** request is recorded as "no
   `session/request_permission` in `default` mode for a write", not failed.
3. **Usage**: every `DriverEvent::Usage` frame is printed with `serde_json::to_string(&usage)` —
   `quota` included once T37 lands (T34 runs first, so the first run prints the raw `_meta` from the
   row's `raw` instead; either is the answer).
4. **Fixture writer**: after the turns, `store.step_events(step_id)` rows with `raw: Some(raw)` where
   `raw["method"] == "session/update"` (the transport's raw is exactly `{ "method", "params" }`,
   `acp/mod.rs:1413`) are written one per line as `{"direction":"agent", …raw}` to
   `$HTUI_AGY_FIXTURE_OUT` when set (`std::env::var`, never written), else printed. Session ids and
   `$HOME` paths are redacted by hand before commit (H-13). Rows with `htui_synthesized` raws
   (`:1088-1092`) are skipped.

The structural floor is `chat_live.rs:115-153` verbatim, with `pgrep -f agy_acp_server`.

### B.11 Single-writer quota columns (T45, D74)

`pg/write.rs:418-436` — `quota` and `quota_at` leave **both** lists, so the statement goes from ten
bound parameters to eight and `row.quota` / `row.quota_at` are no longer read:

```rust
"INSERT INTO agent_box (agent_id, box_id, enabled, version, path, probed_at, updated_at, probe) \
 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
 ON CONFLICT (agent_id, box_id) DO UPDATE SET \
     enabled   = EXCLUDED.enabled, \
     version   = EXCLUDED.version, \
     path      = EXCLUDED.path, \
     probed_at = EXCLUDED.probed_at, \
     probe     = EXCLUDED.probe"
```

A fresh row therefore inserts `quota`/`quota_at` as `NULL` — the column default and the honest value:
a row nobody has latched has no observed allowance, and a probe handshake reports none (§7). `.sqlx`
regenerates.

`mem.rs:839-848` — the conflict arm stops replacing the row wholesale, because `*stored = row.clone()`
is the in-memory spelling of `SET quota = EXCLUDED.quota`:

```rust
Some(stored) => {
    // D74: `quota`/`quota_at` are `set_agent_box_quota`'s alone. Replacing the row here would be
    // this backend's version of the `EXCLUDED.quota` the SQL no longer writes, and the two
    // backends have to agree or the conformance case passes on one and fails on the other.
    let (quota, quota_at) = (stored.quota.clone(), stored.quota_at);
    *stored = row.clone();
    stored.quota = quota;
    stored.quota_at = quota_at;
    stored.updated_at = now;
}
None => {
    let mut fresh = row.clone();
    fresh.quota = None;          // the INSERT list's two missing columns
    fresh.quota_at = None;
    self.agent_boxes.insert((row.agent_id, row.box_id), fresh);
}
```

`probe.rs:1527-1528` — the carry-forward goes, and the doc says why:

```rust
// Was: quota: existing.and_then(|row| row.quota.clone()), quota_at: existing.and_then(|row| row.quota_at),
quota: None,
quota_at: None,
```

with `agent_box_row`'s doc paragraph rewritten: the probe owns neither field **and no longer carries
them**, because since MOD-2 milestone 7 (plan D74) `upsert_agent_box` cannot write them at all —
`set_agent_box_quota` is the only writer, and carrying a value into a statement that discards it was
how a latch got lost. `existing` stays a parameter (`version`'s fallback still reads it — check that
it does; if `existing` becomes unused, drop the parameter and fix its three call sites rather than
leaving a `_existing`).

Docs that must move with the code, or the next reader re-introduces the bug: `AgentBox.quota` /
`.quota_at` (`model/agent.rs:78-81`) name the single writer; `WriteStore::upsert_agent_box`
(`traits.rs:117`) states that the two columns are **not** part of what it writes — an `AgentBox`
carrying them is not an error and not a write, which is the one sharp edge this design keeps and the
conformance case pins.

**MOD-7 inherits this.** `agent_box_row`'s old doc said "MOD-7 writes both"; under D74 MOD-7 writes
them through `set_agent_box_quota` like everything else. The sentence is corrected rather than
deleted, so box registration does not rediscover the missing columns the hard way.

### B.9 The two agent conformance cases and `UsageSpy` (T39, T40)

`UsageSpy` (`conformance.rs:434-540`) gains a second log:

```rust
#[derive(Debug, Clone, PartialEq)]
struct QuotaCall { agent_id: AgentId, box_id: BoxId, quota: Value, quota_at: DateTime<Utc> }
struct UsageSpy<'a, S: WriteStore> { inner: &'a S, calls: Mutex<Vec<UsageCall>>, quota_calls: Mutex<Vec<QuotaCall>> }
fn quota_calls(&self) -> Vec<QuotaCall>;
// set_agent_box_quota: delegate first, log after — the `:519-531` rule, no `.await` under the guard
```

`quota_blob_latches_agent_box` (after `usage_deltas_sum_to_step_usage`):
`store.upsert_agent_box(&AgentBox { agent_id: ids::AGENT_CLAUDE, box_id: ids::BOX, probe: Some(json!({"status":"ready","source":"probe"})), quota: Some(json!({"remaining": 100})), .. })`
first. Script A: `usage(1_000, 100, None)`, `usage(2_000, 250, Some(BLOB))`, `usage(2_100, 1, None)`,
`done(EndTurn)` where `BLOB` is fixture line 5's object (C). Recorder
`.with_quota_latch(QuotaLatch { agent_id: ids::AGENT_CLAUDE, box_id: ids::BOX, source: QuotaSource::AcpMetaRateLimit, billing: Billing::Subscription })`.
Assert `spy.quota_calls()` has **two** entries (row 1 latched nothing: no blob yet; rows 2 and 3
did), the last equals
`normalize(AcpMetaRateLimit, Subscription, Some(&BLOB), Some(351), at3).to_value()` with
`quota_at == at3`, and the second's `spend.session_micros == 350`. Script B: the same three rows with
`quota: None` and `source: QuotaSource::None`, `billing: PerToken` → three calls, every one
`windows: []`, `status: null`, `exhausted: false`, spend `100`, `350`, `351`. Both scripts' `usage`
rows persist with the `quota` key exactly as emitted (`str_at`/`payload["quota"]`).

`run_cap_breach_cancels_within_one_event` (after `cancel_answers_parked_permissions`): an inner
`play(harness, store, cap: Option<i64>, script) -> (Vec<SessionEvent>, RecorderSummary, DoneEvent)`
over `UsageSpy`. (a) cap `300`, script `usage(1_000, 100)`, `usage(2_000, 250)`,
`ScriptEvent::ExpectCancel` → `stop.stop_reason == Cancelled`;
`summary.cap_breach == Some(CapBreach { 300, 350, at2 })`; rows: the breaching `usage` is immediately
followed by `error` (`code == "cap_exceeded"`, role `htui`, message contains `"per_token_cap_run"`),
then `done` with `stop_reason == "cancelled"` as the **last** row — "within one event" is
`log[i+1].kind == Error` for the breaching `i`; `spy.calls().last().usage["cost_micros"] == 350`.
(b) cap `1_000`, `usage(100)`, `usage(250)`, `done(EndTurn)` → no `error` row, `done end_turn`,
`cap_breach == None`. (c) cap `None`, same script → identical rows. (d) cap `50`, `usage(100)`,
`ExpectCancel` → cancelled on the first row. **Red for the stated reason** before T40: (a) and (d)
pull past the breaching row into `ExpectCancel`, which the harness reports as
`Err(DriverError::Transport(_))` ("a case that forgets to cancel fails instead of hanging",
`conformance.rs:107-110`), and `pump(..).expect(..)` panics there.

`CASES` gains both names after `"usage_deltas_sum_to_step_usage"`; `run_case` gains two arms;
`acp_conformance.rs:85` and `fake_conformance.rs:30` assert `15`.

### B.10 `crates/htui/tests/chat_usage_pg.rs` (T43, P-3)

`#![cfg(feature = "testkit")]`-free (it needs no harness); `use htui_store::testkit as common;`.
Fixture path `concat!(env!("CARGO_MANIFEST_DIR"), "/../htui-agent/tests/fixtures/claude_acp_turn.jsonl")`;
the `replay.rs:406-417` mapper loop.

1. `an_acp_step_usage_equals_the_last_cost_total_on_postgres`: `demo_db()` or `SKIP`;
   `ChatRunSpec::mint(ids::PROJECT_HTUI, db.store.this_box(), ids::USER, Some(ids::AGENT_CLAUDE), None)`;
   `Writer::Online(db.store.clone()).start_chat_run(&chat)`;
   `Recorder::new(&writer, &scrubber, chat.step_id, false, None)`; `record_prompt`; every mapped event
   through `record`; `finish`; then `SELECT usage FROM run_step WHERE id = $1` (the
   `pg_criteria.rs:98-105` raw read) → `usage["cost_micros"]` equals the maximum `cost_micros_total`
   among the persisted `usage` rows **and** equals `UsageTotals::from_rows(&rows).to_value()["cost_micros"]`
   — both clauses of criterion 7 in one place, on a real ACP transcript, on Postgres. `db.drop_db()`.
2. `the_latch_lands_on_postgres_and_leaves_probe_alone`: same recorder with `.with_quota_latch(..)`
   after `db.store.upsert_agent_box(&probed_row)`; afterwards
   `SELECT quota, quota_at, probe FROM agent_box WHERE agent_id = $1 AND box_id = $2` → `probe`
   equals the inserted document byte for byte (`Value` equality), `quota` parses through
   `Quota::from_value` with `windows` ids `["five_hour", "seven_day"]`,
   `quota_at == quota.observed_at`.

---

## C. The blob, worked

Fixture line 5 (`claude_acp_turn.jsonl`), the `_meta["_claude/rateLimit"]` value the mapper now lifts
onto `UsageEvent.quota` verbatim:

```json
{"status": "allowed", "resetsAt": 1788801600, "rateLimitType": "five_hour",
 "overageStatus": "rejected", "overageDisabledReason": "org_level_disabled", "isUsingOverage": false,
 "unifiedWindows": {"five_hour": {"utilization": 0.11, "resetsAt": 1788801600},
                    "seven_day": {"utilization": 0.62, "resetsAt": 1788854400}}}
```

`normalize(AcpMetaRateLimit, Subscription, Some(&blob), Some(351), at)` → `to_value()`:

```json
{
  "source": "acp_meta_rate_limit",
  "billing": "subscription",
  "status": "allowed",
  "exhausted": false,
  "windows": [
    { "id": "five_hour", "utilization": 0.11, "resets_at": "2026-09-07T17:20:00Z" },
    { "id": "seven_day", "utilization": 0.62, "resets_at": "2026-09-08T08:00:00Z" }
  ],
  "spend": { "session_micros": 351, "currency": "USD" },
  "observed_at": "<at, RFC 3339>"
}
```

(The two `resets_at` values are `DateTime::from_timestamp(1788801600, 0)` and `(1788854400, 0)` —
derive them in the test with that call rather than copying the strings above.) `overageStatus`,
`rateLimitType`, `isUsingOverage` are **not** normalized: they stay on the `usage` row's `quota` key,
which is D66's reason for keeping the raw blob ("a vendor field this milestone does not understand is
still recorded"). The seed `agy` row (`source: "none"`, `billing: subscription`) with a costed usage
row latches
`{ "source": "none", "billing": "subscription", "status": null, "exhausted": false, "windows": [], "spend": { … }, "observed_at": … }`;
with no cost it latches nothing at all (B.5's "nothing to say" rule), which is D65's "`agy`'s quota
column reads `—` by design".

`available(Some(&doc), Some(351), Some(300))` → `Skip(CapReached { 351, 300 })`; with `cap: None` →
`Available`; with `status: "allowed_warning"` → `Skip(Status("allowed_warning"))` (H-7);
`available(None, _, None)` → `Available`; `available(Some(&json!({"remaining": 100})), None, None)` →
`Available` (does not parse → unknown).

---

## D. Data flow

### D-1. The latch (T39)

```
adapter ──usage_update{used,size,cost,_meta}──▶ Mapper::usage → UsageEvent{cost_micros Δ, cost_micros_total, quota: Some(blob)}
  → run_turn → Recorder::record
      scrub_envelope: round_trip(UsageEvent) → (typed, payload)      the blob is inside the payload: scrubbed at capture, again at flush
      usage.add_payload(&payload)                                    the five keys; `quota` is not one (usage.rs:48-54)
      latch_quota(&payload, at):
        payload.quota? → last_quota_raw
        nothing to say? → return                                     (no blob this session, no spend)
        normalize(source, billing, last_quota_raw, usage.cost_micros, at) → set_agent_box_quota(agent, box, doc, at)
          Memory/Online: UPDATE two columns · NotFound → latch off for the session (debug)
          Buffered: never reached — quota_latch_for() answered None at start() with REGISTRY_ON_SERVER_ONLY logged (D68)
      check_cap(at) → None
      push row {kind: usage, payload with `quota`}                   the row keeps the vendor blob verbatim
```

### D-2. The breach (T40)

```
Recorder::record(usage row N): usage.cost_micros ≥ run_cap.micros → cap_breached = Some(..) → Ok(Some(CapBreach))
  pump / run_turn:
    enforce_breach(session, recorder, breach):
      session.cancel(grace)          ACP: parked answered `cancelled`, session/cancel, grace window, close_turn(Cancelled), kill
                                     Fake: synthesized tool_results, Done(Cancelled) queued
      while next_event: Done? → withheld, break · else record (verdict None now)
      record_cap_breach(breach, transport_done):
        rows N+1 = error{cap_exceeded} (htui) · N+2 = done{cancelled} (raw/at from the withheld done) · flush · sync_step · 2 UI frames
      → DoneEvent{Cancelled}
    run_turn → forward(ui) → TurnEnd::CapExceeded
  run_chat: status = Failed, last_stop = Cancelled → finish() → close_run(Failed) → frames.ended(Cancelled)
```

Criterion 8's "within one event": the cancel is issued before another event is pulled, and the two
closing rows follow whatever the cancel itself produced; the breaching `usage` row and `error` are
adjacent unless the transport's cancel synthesized a `tool_result` for an open call, in which case
that row sits between them and the **last two** rows are still `error`, `done` — which is what
criterion 8 asserts. The conformance case pins adjacency with no open call; F-T40's recorder case
pins the open-call ordering.

---

## E. Build order, per task, TDD

Order: **T34 → (T37 ∥ T38→T45 ∥ T41) → T39 → (T40 ∥ T42 ∥ T43) → T44.** T45 (D74, single-writer
quota columns, B.11) is serial immediately after T38: same four store files, same implementer, and
T38's `set_agent_box_quota` has to exist before anything can prove that it is the *only* writer. Every checkpoint starts with
`cargo fmt --all -- --check`; Postgres lines carry
`USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres`; `.sqlx`
steps run from inside `crates/htui-store` with
`DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features`
(README `:382-401`).

| Pair | Intersection | Parallel? |
|---|---|---|
| T37 × T38 | ∅ (T37: `event.rs`, `acp/map.rs`, `tests/acp_map.rs`, two snapshots; T38: store crates, `traits.rs`, the two spies' delegating arms, `.sqlx`) | **yes** |
| T37 × T41, T38 × T41 | ∅ (`settings/agents.rs`, `tests/settings.rs`, snapshots) | **yes** |
| T39 × T37/T38 | `record.rs` reads `UsageEvent.quota` (T37) and calls `set_agent_box_quota` (T38) | serial after both |
| T40 × T42 | ∅ once `ProjectCaps` is in T39's slice of `quota.rs` (T40 touches `record.rs`, the worker, the four read-path files; T42 only `quota.rs`) | **yes** |
| T40 × T43 | ∅ (`chat_usage_pg.rs` is new) | **yes** |

### T34 — the live `agy` turn (first, alone)

| # | Step | Proof |
|---|---|---|
| 1 | `chat_live_agy.rs` per B.8 and G-T34; run it with the fixture variable set | `HTUI_KEEP_RAW_EVENTS=1 HTUI_AGY_FIXTURE_OUT=/tmp/agy_acp_turn.jsonl cargo test -p htui --features testkit --test chat_live_agy -- --ignored --nocapture` |
| 2 | Redact and commit the fixture; `acp_map.rs` snapshot case; the seed edit per D64 **only** if `session/new` supplied models | `cargo test -p htui-agent --features test-support --test acp_map`; `cargo insta review` |
| 3 | Re-run `agy_live.rs` case 3 for D64 | `cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture` |
| 4 | Write the three answers into the plan's answer table (T44 copies them to the decision doc) | — |

Commit: `test(tui,agent): live agy chat, recorded transcript and the §11.14 answers`.

### T37 — `_meta` capture

1. **Red**: `tests/acp_map.rs::the_rate_limit_blob_is_captured_verbatim_and_never_summed` — over
   `mapped("claude_acp_turn.jsonl")`, the first `Usage` has `quota == None`, some later `Usage` has
   `quota == Some(blob)` equal to the fixture line's `_meta["_claude/rateLimit"]`, and
   `UsageTotals::default().add_payload(&serde_json::to_value(&usage))` changes `cost_micros` only.
   **Fails because** `UsageEvent` has no `quota` field (compile error, the honest red for a struct
   addition).
2. `event.rs` field; `map.rs` constant and the three-line fill.
3. `cargo insta review` for `acp_map__turn` and `replay__acp_turn`;
   `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings`.

Commit: `feat(agent): capture the ACP rate-limit blob onto UsageEvent.quota (D66)`.

### T38 — the narrow setter

1. **Red**: store case `set_agent_box_quota_updates_two_columns_or_not_found` (B.3) and `mem.rs`
   in-module `set_agent_box_quota_leaves_probe_and_version_alone` (upsert with `probe: P`,
   `version: Some("1.2.3")`; set quota; `agents()` on a store whose `this_box == ids::BOX` shows
   `probe == P`, `version` unchanged, `quota`/`quota_at` new, `updated_at` moved). **Fails because**
   no such method on `WriteStore`.
2. `traits.rs`; `mem.rs`; `pg/write.rs` + `.sqlx`; `writer.rs` both arms; the two spies' delegating
   arms; `pg_conformance.rs` → 21.
3. `pg_criteria.rs::set_agent_box_quota_leaves_probe_byte_identical`: `demo_db()`; insert a row
   through `upsert_agent_box` with a 200-byte `probe` document; setter;
   `SELECT probe, quota, quota_at FROM agent_box …` → `probe` `Value`-equal, the two columns equal
   the call, and `SELECT updated_at` moved (or not — H-11 decides which assertion).
   `writer_buffered.rs` gains `set_agent_box_quota` to the
   `item_and_registry_writes_are_unreachable` list.

Proof: `cargo test -p htui-core --all-features`; Postgres line for `pg_conformance` and
`pg_criteria`; `cargo sqlx prepare --check`.
Commit: `feat(store): WriteStore::set_agent_box_quota, two columns and never probe (D67)`.

### T45 — single-writer quota columns (serial after T38; D74, B.11)

1. **Red**: store conformance `upsert_agent_box_cannot_write_quota` (`CASES` 21 → 22,
   `EXPECTED_CASES` → 22): latch through `set_agent_box_quota`, then `upsert_agent_box` the same row
   carrying a **different** `quota` and `quota_at: None` → the stored pair is still the latch's; and a
   first-time `upsert_agent_box` carrying `quota: Some(..)` stores `None`. `pg_criteria.rs` proves
   both with a raw `SELECT` (the `EXCLUDED.quota` line is what is being removed, so the proof has to
   be SQL-level, not trait-level). `crates/htui-agent/tests/probe.rs`:
   `agent_box_row_does_not_carry_quota_forward` — `existing` carries both, the projection returns
   `None`/`None`, every other projected field unchanged. **Fails because** today all three write or
   carry the value.
2. B.11: the two SQL lists, the `MemStore` conflict and insert arms, `agent_box_row`, the four doc
   sites (`AgentBox`'s two fields, `upsert_agent_box`, `agent_box_row` incl. the MOD-7 sentence),
   `.sqlx` regenerated.
3. Check whether `existing` is still read by `agent_box_row` (`version`'s fallback). If it is now
   unused, remove the parameter and fix its call sites — do not leave `_existing`.

Proof: `cargo test -p htui-core --all-features`; `cargo test -p htui-agent --features test-support`;
Postgres line; `cargo sqlx prepare --check`; Windows clippy line.
Commit: `fix(store,agent): quota is single-writer — an upsert cannot discard a latch (D74)`.

### T41 — the Settings quota column

1. **Red**: `tests/settings.rs::the_quota_column_renders_windows_spend_and_nothing` — three
   `probed_row`s (`:277-312`) with `on_box.quota` set to (i) C's document (two windows, `seven_day`
   at `0.62`), (ii) `{ "source":"none", …, "windows": [], "spend": { "session_micros": 394692, "currency": "USD" } }`,
   (iii) `None`, plus (iv) the fixture-shaped `{ "remaining": 100 }`; rendered lines carry
   `62% to 09-08 08:00`, `$0.39 spent`, `—`, `—`; `insta::assert_snapshot!("agents_quota", rendered)`.
   And `the_idle_hint_says_r_cannot_refresh_quota`: the rendered section's last line contains
   `r cannot refresh`. **Fails because** the table has no such column and the hint has no such words.
2. `quota_cell`, column, widths, `HINT_IDLE`;
   `INSTA_UPDATE=always cargo test -p htui --features testkit --test settings`, then without the
   variable; review the four moved snapshots line by line (only the header, one column and the hint
   may differ).

Commit: `feat(tui): a quota column in Settings › Agents, and r cannot refresh it (D73)`.

### T39 — the blob and the passive latch (after T37, T38)

1. **Red**, `htui-core` in-module (`quota.rs`):
   `normalize_lifts_status_windows_and_spend_from_the_claude_blob` (C, windows sorted, epoch →
   timestamp), `a_source_that_reports_nothing_normalizes_to_spend_only`,
   `exhausted_is_derived_from_rejected_or_a_full_window`,
   `to_value_matches_the_serde_form_and_round_trips`,
   `from_value_refuses_a_pre_milestone_document`,
   `project_caps_read_integers_reject_the_rest_and_treat_absent_as_unbounded` (absent → `None`; `0` →
   `Some(0)`; `-1`, `1.5`, `"300"`, `true` → `Err` naming the key). **Fails because** the module does
   not exist.
2. **Red**, `tests/recorder.rs`: `SpyStore` records `set_agent_box_quota` and can be told
   `refuse: Option<StoreError>`; cases `a_usage_row_carrying_a_blob_latches_the_seven_key_document`
   (`quota_at == at`), `a_bare_first_row_latches_nothing_and_a_later_one_keeps_the_last_blob`,
   `a_row_declaring_source_none_latches_spend_only`,
   `a_refused_latch_neither_fails_the_turn_nor_retries` (`Unreachable` → one call, then none;
   `NotFound` likewise; a `Backend` error → a call per row and `Ok`),
   `a_masked_usage_row_is_summed_but_not_latched` (the `record_unreadable` path: a `SessionSpec.env`
   value equal to a field name). **Fails because** `Recorder` has no `with_quota_latch`.
3. **Red**, agent conformance `quota_blob_latches_agent_box` (B.9) — fails on the missing builder; the
   ACP run additionally fails until P-10's encoder lands.
4. `quota.rs` (T39 slice), `mod.rs`; P-6's move (`launch.rs`, `rg QuotaSource crates/` for anything
   the re-export does not cover); `record.rs` latch; `UsageSpy.quota_calls`; the `_meta` encoder;
   `CASES` gains both names now with `run_cap_breach_cancels_within_one_event`'s arm panicking
   `"lands in T40"`, so both count guards are edited once.

Proof: `cargo test -p htui-core --all-features`; `cargo test -p htui-agent --features test-support`
(`recorder`, `fake_conformance`, `acp_conformance`, `launch`, `extensibility`); Windows clippy line;
`cargo doc --workspace --no-deps`.
Commit: `feat(core,agent): the §7 quota document and the recorder's passive latch (D66–D68)`.

### T40 — the run cap (after T39)

1. **Red**, conformance `run_cap_breach_cancels_within_one_event` (B.9) — red for the `ExpectCancel`
   reason stated there.
2. **Red**, `tests/recorder.rs`:
   `a_breach_is_reported_once_and_the_closing_rows_are_error_then_done` over the file's scripted
   `AgentSession` and `enforce_breach` (with one open `tool_call` so the synthesized `tool_result`
   sits between the breaching row and the `error` — D-2's ordering);
   `a_transport_done_that_says_end_turn_is_still_recorded_cancelled` (the session answers the cancel
   with `Done(EndTurn)`); `no_cap_and_context_only_usage_never_breach` (`cost_micros: None` rows
   against `cap: 0`); `a_cap_of_zero_cancels_on_the_first_costed_row`. **Fails because** `record`
   returns `()`.
3. **Red**, `agent_worker.rs` in-module: `fixture_with_project_settings(script, settings)` (a
   `demo_data()` whose `PROJECT_HTUI` row's `settings` is replaced; `DemoData.projects` is `pub`,
   `fixtures.rs:284`); `a_chat_over_its_run_cap_is_cancelled_and_its_run_fails`
   (`{"per_token_cap_run": 300}`, script `usage(100)`, `usage(250)`, `ExpectCancel`; replies hold an
   `Event` frame whose event is `Error { code: "cap_exceeded" }` then one with `Done(Cancelled)` then
   `Ended { Cancelled }`; `active_runs` back to `before`; `step_events` last two rows as criterion 8);
   `a_negative_run_cap_refuses_the_chat_start` (`-1` →
   `Served::Reply(Failed { request: "chat_start", message })` naming `per_token_cap_run`, and
   `runtime` holds no live chat); `a_batch_cap_is_read_and_not_enforced`
   (`{"per_token_cap_batch": 1}` runs to `done end_turn`); `a_buffered_writer_gets_no_latch`
   (`quota_latch_for(&Writer::Buffered(..), ..) == None`, the
   `an_offline_backend_refuses_the_probe_before_spawning_anything` mirror setup at `:3831-3839`).
   **Fails because** `Backend` has no `project_settings`.
4. `record.rs` cap slice; `pump`; the read path (mem, pg + `.sqlx`, cache, backend); the worker.

Proof: `cargo test -p htui-agent --features test-support`; `cargo test -p htui --features testkit`;
Postgres line (`pg_conformance` for the `.sqlx`); Windows clippy line.
Commit: `feat(agent,tui): the per-run cap — detected in the recorder, cancelled by the loop that holds the session (D69, D70)`.

### T42 — `R-AGT-8`'s predicate (after T39, ∥ T40)

1. **Red**, `quota.rs` in-module: one case per rule (`exhausted`; a window at `1.0`;
   `status: "rejected"`; `status: "allowed_warning"` → `Status`; `spend 300, cap 300` →
   `CapReached`) and the two that must not skip (`None` quota with `spend 1, cap None`; a window at
   `0.99` with `status: "allowed"`), plus `an_unparsable_blob_is_unknown_and_available`. **Fails
   because** `available` does not exist.
2. `Availability`, `SkipReason`, `available`; re-exports.

Commit: `feat(core): quota::available, R-AGT-8's skip predicate for MOD-4 (D72)`.

### T43 — criterion 7 on Postgres (after T39, ∥ T40)

`chat_usage_pg.rs` per B.10. **Red** only in the sense every Postgres test is: it is a proof, and it
goes red if the mapper's total and the recorder's sum ever part. Proof: the Postgres line.
Commit: `test(tui): criterion 7's ACP clause and the latch against live Postgres`.

### T44 — close-out

README (row 29), HANDOFF phase-7 note, PRD rows, the answer table;
`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`; the plan's full Validation
block including both live lines. Commit: `docs(mod-2): milestones 6 and 7 close-out`.

---

## F. Hazards

| # | Failure mode | Detected by | Closed by |
|---|---|---|---|
| H-1 | A **partial or foreign-shaped `usage_update`**: no `cost`, a `cost` in EUR, `_meta` present but `unifiedWindows` missing or a window without `utilization`. The mapper already leaves `cost_micros` `None` on non-USD (`map.rs:98-103`); a naive normalizer would panic on a missing key or write a window with `utilization: 0` | F-T39 `normalize_*` cases with each key removed in turn; `a_non_usd_cost_is_reported_as_it_arrived_and_sums_nothing` (`map.rs:712`) | `normalize` reads every vendor key through `get`/`as_*` and skips a window without a numeric `utilization`; `Spend.session_micros` stays `None` on non-USD, so the blob says "no USD spend" rather than a wrong number |
| H-2 | **A breach detected on a row the store then refuses.** `flush` keeps refused rows in `unflushed` and re-offers them at the same `seq` (`record.rs:637-642`, `:676-679`). The breaching row's `add_payload` and `check_cap` have already run when its flush fails: the verdict is out, `enforce_breach` runs, and `record_cap_breach`'s own `flush` re-offers `unflushed` (the usage row) **ahead of** the two closing rows — the order is preserved by construction (`rows = take(unflushed)` then the buffer, `:651-653`). If that flush fails too, `RecordError::Store` propagates from `enforce_breach`, `run_chat` marks the run `Failed` and the closing rows stay owed in `unflushed` for `finish` | F-T40 recorder case with `SpyStore` refusing the breaching flush once (`refuse_next_append`) then accepting: rows come out `usage(N)`, `error`, `done`, gapless | The re-offer rule the recorder already has; the case pins that a breach never double-counts (`add_payload` ran once) |
| H-3 | **The first `usage_update` of a turn carries no `_meta`** (`acp_map.rs:110-112`), so a latch-on-every-row design would erase last session's windows with an empty document on every turn's first report | F-T39 `a_bare_first_row_latches_nothing…`; conformance script A expects **two** calls, not three | `last_quota_raw` + the "nothing to say" rule (B.5): no blob seen this session and no spend → no write |
| H-4 | **Cancel racing a `done` already in flight.** The breaching row is followed on the wire by the agent's own `done{end_turn}` (ACP `cancel_session` step 3 records a `StopReason` that arrives inside the grace window as the turn's real end, `acp/mod.rs:1687-1707`; the fake queues `Done(Cancelled)`). Recording that `done` would put it **before** the `error` row or make the last row `end_turn` | F-T40 `a_transport_done_that_says_end_turn_is_still_recorded_cancelled`; conformance (a) on both transports | `enforce_breach` withholds the transport's `Done` and `record_cap_breach` writes `done{cancelled}` carrying its `raw`/`at` (B.5). Exactly one `done` per turn (§4.1) still holds |
| H-5 | **Context-only usage** (`used`/`size`, no `cost` — `agy`'s expected shape, and the first rows of every `claude` turn) against a cap: `cost_micros` is `None`, so a comparison written as `unwrap_or(0) >= cap` would fire on a cap of `0` for a session that spent nothing | F-T40 `no_cap_and_context_only_usage_never_breach` | `check_cap` returns `None` while `usage.cost_micros` is `None`; a cap of `0` cancels on the first **costed** row (B.1's `from_settings` doc) |
| H-6 | **The latch firing on a row re-probed mid-session.** `run_reprobe` writes the whole row through `upsert_agent_box` carrying the `quota`/`quota_at` it read at `start()` (`probe.rs:1527-1528` carries the old value forward; `pg/write.rs:426-427` writes both), so every latch between the re-probe's read and its write is discarded — a lost update by construction, not a timing accident | **Tested** as of T45: the store conformance case, the `pg_criteria.rs` SQL-level pair, and the `agent_box_row` projection case (B.11) | **CLOSED by D74** (maintainer's instruction, 2026-09-10), no longer accepted: the two columns become single-writer — `upsert_agent_box` cannot write them on either path, `MemStore` preserves them, and the probe stops carrying them forward. B.11 has the change |
| H-7 | `claude`'s `status` vocabulary includes `allowed_warning`; §7's rule "status is not `allowed`" makes MOD-4 skip a warned-but-allowed agent | F-T42 pins it as `Skip(Status("allowed_warning"))` | Deliberate: §7's rule, applied as written. Stated in `available`'s doc so MOD-4 can loosen it knowingly |
| H-8 | `record` changes return type; `let () = …` or `.map(|()| …)` on any of its 21 call sites would break | The compiler; `rg 'record\(' crates/` before T40 shows every site is `.await?;`, `.await.expect(..)` or `if let Err(..)` — none binds the `()` | `Option<CapBreach>` is not `#[must_use]`, so the existing `.expect(..)` statements compile unchanged |
| H-9 | `pump`'s cancel needs a grace; adding a parameter moves 18 call sites | `RunCap.grace` (B.5): zero in the suite (matching `cancel_answers_parked_permissions`'s `Duration::from_millis(0)`), `CANCEL_GRACE` in the worker | Signature unchanged; `run_cap()` exposes it to `enforce_breach` |
| H-10 | The **ACP conformance harness** encodes `Usage` without `_meta` (`acp_conformance.rs:355-368`), so the latch case passes on the fake and fails on ACP — criterion 1 broken the other way round | `the_acp_transport_passes_every_case` after T39 | P-10's encoder line, keyed on `RATE_LIMIT_META_KEY` |
| H-11 | `agent_box.updated_at` on the narrow `UPDATE`: the mem arm bumps it, the pg arm relies on the `BEFORE UPDATE` trigger loop (`0001_init.sql:578`) which may or may not list `agent_box` | `pg_criteria.rs::set_agent_box_quota_leaves_probe_byte_identical` asserts `updated_at` moved | **UNVERIFIED** (B.3): if `agent_box` is not in the loop, `SET … , updated_at = now()` |
| H-12 | The eighth column at 100 columns: 55 fixed + 19 + `Min(11)` + gaps = 92 plus whatever border the section draws | The four re-accepted snapshots show a truncated header if it does not fit | Shrink `default` `9 → 8` or `quota` `19 → 16` (`"62% to 09-08"` without the time) — the snapshot decides, the implementer picks the first that fits and says so in the commit |
| H-13 | The `agy` fixture carries `$HOME` paths and a session id | Hand review before commit | Redact as `claude_acp_turn.jsonl` was (`acp_map.rs:5-8`); the snapshot case asserts kinds, not paths |
| H-14 | `set_current_dir` in T34 is process-wide | The file is one `#[ignore]` test in its own binary | The `Drop` guard restores it; documented in the module doc |
| H-15 | T34 finds `agy_acp_server` **does** emit a usage or quota shape this mapper does not read | The printed frames | A finding for the maintainer, not a quiet fix: a new `QuotaSource` value or a mapper arm is a D62/D66-class change and goes into the plan's answer table first |
| H-16 | **Windows-only compile surface.** `record.rs`, `quota.rs` and the worker changes carry no `cfg`; `RunCap.grace` is a `Duration`; nothing touches `launch.rs`'s process code | ~~`cargo clippy --target x86_64-pc-windows-msvc …`~~ — **the detector does not run on this box.** Re-confirmed 2026-09-10 (T37, then independently): it dies in `ring`'s build script with `error occurred in cc-rs: failed to find tool "lib.exe"`, before any `htui` code is compiled. `HANDOFF.md` TOOL-3; same state MOD-20/MOD-21 recorded | No Windows-specific code is added, so the hazard's *exposure* is nil — but the guard is **unavailable**, not green, and the plan's acceptance list says so. MOD-16 inherits it. The line stays in the validation block for the first box with an MSVC toolchain |
| H-17 | `latch_quota` awaits a store write **inside** `record`, before the row is buffered: a slow Postgres makes every `usage` row cost a round trip on the chat's task | Not the UI task (`R-NF-3` holds); a `usage` row is rare (one per report) | Accepted; the write is one indexed `UPDATE`. If it ever matters, the latch moves to `sync_step`'s cadence — same document, fewer writes |
| H-18 | `quota_cell` parses `resets_at` with `DateTime::parse_from_rfc3339` on the UI task | A malformed string renders the percentage alone, never panics | `ok()` on the parse |

---

## G. Open questions for the implementer

1. **T34's three answers are inputs** (D65). If `agy_acp_server` emits `usage_update` with a `cost`,
   the seed's `billing`/`quota.source` are unchanged (the source is still `"none"`: a cost is spend,
   not an allowance) and its quota column shows `$x.xx spent`. If it emits a vendor allowance blob
   under any key, stop: that is H-15.
2. ~~**The re-probe's `upsert_agent_box` carrying stale `quota`** (H-6)~~ — **answered by the
   maintainer on 2026-09-10: fix it now.** It is plan D74 and task T45, specified in B.11. The
   `COALESCE` alternative was rejected in favour of removing the two columns from the statement
   outright: `COALESCE(EXCLUDED.quota, agent_box.quota)` would still let an upsert *set* the column
   and would make clearing it impossible, which is two surprises instead of none.
3. **`ChatFrame::Failed` on a cap breach**: this blueprint sends `Ended { Cancelled }` only, on D73's
   reading that the `error` row is the visibility. If the reviewer wants the tab's header to say
   "failed", it is one `frames.failed(message)` before the `break` in `run_chat`'s `CapExceeded` arm
   and one snapshot.
4. **H-11** (`agent_box` in the trigger loop) and **H-12** (column width) are checked in T38 and T41
   respectively; both have a stated fallback.

**Plan claims that were false against the tree**, stated plainly and now amended in the plan itself:
the file-table row that put "two new `CASES` entries (13 → 15)" in
`crates/htui-core/src/store/conformance.rs` (P-2 — that file has 20 cases and no `agent_box` read);
T43's placement in `crates/htui-store/tests/pg_criteria.rs` (P-3 — that crate cannot drive an ACP
session); and D69's mechanism as the plan states it (P-1 — `pump` is not the production loop,
`run_turn` is). None changes a decision; each changes a file.

---

## H. What this milestone does NOT touch

- **No migration**, no `cache_migrations` change, no new mirrored table: `agent_box` stays
  server-side (D68), `project.settings` is already mirrored.
- **No `DriverEvent` variant, no `EventKind`, no `ChatFrame`/`StoreRequest`/`StoreReply` variant**:
  the blob is a key on `UsageEvent`, the breach is two existing kinds, the caps ride `ChatArgs`.
- **No `ReadStore` method**: `project_settings` is inherent per store with a `Backend` dispatch, the
  `agents()` shape; `pg_conformance.rs` changes its count only.
- **The Chat tab** (D73): no cost widget; `usage_line` (`transcript.rs:569`) is unchanged and does
  not read `quota`.
- **The batch cap** (D71): read, logged, never compared.
- **The CLI transport** (milestone 8): `QuotaSource::CliRateLimitEvent` / `CliStatusLine` normalize
  to "no status, no windows" until a reader exists; `--max-budget-usd` is not passed anywhere.
- **`Recorder::new`'s signature** and the transport-neutrality of the suite: the recorder gains two
  builders and no session.
- **`probe.rs`** (H-6's durable fix is not made here) and **`registry.rs`**: nothing gains an arm on
  `agent.name`; `tests/extensibility.rs`'s sweep stays green.
