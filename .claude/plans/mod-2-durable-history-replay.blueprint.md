# Blueprint: MOD-2 milestone 4 — durable history and replay

**Plan**: `.claude/plans/mod-2-durable-history-replay.plan.md` (D31–D42, T15–T21 binding; the
"Verified claims" table is taken as read and nothing there is re-verified here).
**Design authority**: `docs/ANA-4.md` §4.1 (recorder rules, the offline JSON-lines buffer), §4.4
("`htui` replays transcripts from its own store"), §6.1, §9 step 4, §11 criteria 2, 3, 4, 7, 12;
`docs/ANA-9.md` §4.3, §4.4. Where the plan and the tree disagree the tree wins and the disagreement
is marked **plan ≠ tree** at the point it bites. Anything not verified in source is marked
**UNVERIFIED — implementer must check** and is never invented.

Conventions this milestone inherits unchanged: `#![warn(missing_docs)]` on every lib,
`unsafe_code = "forbid"`, the workspace lint set (`clippy::all`, **not** `pedantic`, so an `Option`
every arm fills is not a lint), MSRV 1.98, one `thiserror` type per crate, no lock across an
`.await`, `htui-store` never depends on `htui-agent` (build **or** dev), the scrubber stays in the
recorder (`crates/htui-store/src/cache/pending.rs:9-12`: "`htui-store` never inspects a payload"),
and no store handle on the render side (`R-NF-3`: replay arrives as a `StoreReply`, decode is over
the rows the reply carries).

Two things the plan settles that the tree contradicts, both resolved in the tree's favour:

- **plan ≠ plan.** The file table's `cache/mod.rs` row says "the nil-project cursor constant
  (D32)"; D32 itself (rewritten after the falsified claim) says `agent` is a full replace with **no**
  cursor row. D32's text governs: no constant, no cursor, `replace_agent` beside `replace_app_user`.
- **plan ≠ tree.** D39's "`j`/`k` inside the selected run": the Backlog tab consumes `j`/`k` for
  its own list cursor before the detail pane sees any key (`ui/tabs/backlog/mod.rs:201-202`), and
  the detail pane's vertical keys are already `J`/`K` (`detail/mod.rs:274-275`). Step selection is
  `J`/`K`; `Enter` replays. The Backlog tab also consumes `Enter` for folding and returns `Pass`
  without offering it to the pane (`backlog/mod.rs:207`, `fold()` at `:143`), so `backlog/mod.rs`
  is a **file-set addition** to T19 (one arm: an `Enter` that did not fold is offered to the pane).

One deviation is proposed, not assumed — **H-1**, a race the plan does not name (a chat that
outlives the reconnect is uploaded mid-flight and its `run_step.usage` frozen at a partial sum). The
fix changes D35's `finish_chat_run` from "no-op that logs" to "seals the buffer file" and adds two
functions to `pending.rs`. It is built into sections B.6/B.8 and G below **as the design**; if the
maintainer vetoes it at review, the three items marked `[H-1]` are dropped and the race is recorded
as a known gap instead.

---

## A. Module map (build order)

| # | File | Action | Task | Responsibility (one line) |
|---|---|---|---|---|
| 1 | `crates/htui-store/cache_migrations/0002_agent_mirror.sql` | CREATE | T15 | The mirrored `agent` table, B.1. Picked up by `CACHE_MIGRATOR` (`lib.rs:42`, `sqlx::migrate!("./cache_migrations")`) at the next `CacheStore::open`, which runs the migrator **before** it reads `cache_meta` (`cache/mod.rs:122-125`), so an existing mirror gains the table in place — no rebuild, `cache_meta.schema_version` (the *Postgres* version) untouched. |
| 2 | `crates/htui-store/src/cache/mod.rs` | UPDATE | T15 | `MIRRORED_TABLES: [&str; 16]`, `"agent"` inserted after `"app_user"` (index 1); doc "sixteen". `rebuild()`'s `DELETE` loop (`:164`) and the three `tests/cache.rs` rebuild cases that iterate the array (`:827`, `:874`, `:924`) cover it for free. `open()` additionally calls `pending::seal_orphaned(&dir)` after creating `pending/` `[H-1]`. |
| 3 | `crates/htui-store/src/identity.rs` | UPDATE | T15 | `pub fn os_user_name() -> String` — the `USERNAME` / `USER` / `"htui"` rule moved out of `pg/mod.rs::seed_user_name` (`:472-481`) so D33's "the same name" is one function, not two copies. **File-set addition** (T15 lists neither this file nor `pg/mod.rs`). |
| 4 | `crates/htui-store/src/pg/mod.rs` | UPDATE | T15 | Delete the private `seed_user_name`, call `identity::os_user_name()` at the one site. **File-set addition**, mechanical. |
| 5 | `crates/htui-store/src/cache/read.rs` | UPDATE | T15 | `CacheStore::agents()`, `CacheStore::this_user()`, `CacheStore::user_named()`, B.3. First reader of `bool_col` — the `#[expect(dead_code)]` at `:107-110` comes off. |
| 6 | `crates/htui-store/src/cache/refresh.rs` | UPDATE | T15 | `AGENT_COLUMNS`, `replace_agent`, called right after `replace_app_user` in `run_pass` (`:243`); the step-1 doc line (`:216-217`) names `agent`. A new `query!` → `.sqlx/` regenerates in **T15**, not only T17 (**plan gap**: T15's file set lacks `.sqlx/`). |
| 7 | `crates/htui-store/.sqlx/` | UPDATE | T15, T17 | One new query file per `query!` (T15: the `agent` select; T17: the widened `run_step` insert). |
| 8 | `crates/htui-store/src/backend.rs` | UPDATE | T15, T16 | T15: the `Offline` arms of `agents()` / `this_user()` go to the cache; the `agents()` doc (`:251-266`) is rewritten. T16: `writer()` answers `Some(Writer::Buffered(..))` for `Offline`; module doc (`:6-12`) amended, B.5. |
| 9 | `crates/htui-store/src/cache/pending.rs` | UPDATE | T16, T17 | T16 `[H-1]`: `OPEN_SUFFIX`, `append_pending` writes the `.open` name, `seal_pending`, `seal_orphaned`; `list()` unchanged (its extension filter already skips `.open`). T17: `upload_one` fills `run_step.prompt_digest` and `run_step.usage` (B.8). **T16 gains this file** (the naming contract has one owner, `:9-10`), so **T16 → T17 is serial** (plan says T17 ∥ T18 only; that still holds). |
| 10 | `crates/htui-store/src/writer.rs` | UPDATE | T16 | `Writer::Buffered`, `BufferedWriter`, its `ReadStore` / `WriteStore` impls, `label()` → `"buffered"`, module doc rewritten (B.6). |
| 11 | `crates/htui-store/tests/writer_buffered.rs` | CREATE | T16 | The buffered writer over a throwaway mirror; no Postgres (F-2). |
| 12 | `crates/htui-store/tests/cache.rs` | UPDATE | T15, T16, T17 | T15: the registry pass + offline reads + `user_named`. T16: `seal_pending` / `seal_orphaned` file-only cases beside `pending_append_*` (`:1217-1300`). |
| 13 | `crates/htui-core/src/model/usage.rs` | CREATE | T17 | `UsageTotals` + `from_rows` + `add_payload` + `to_value` (B.7). |
| 14 | `crates/htui-core/src/model/mod.rs` | UPDATE | T17 | `pub mod usage; pub use usage::UsageTotals;`. |
| 15 | `crates/htui-agent/src/record.rs` | UPDATE | T17 | Private `UsageTotals` (`:202-241`) and `add_delta` deleted; `Recorder.usage: htui_core::model::UsageTotals`; the sum call moves to `add_payload(&payload)` (B.7 explains why). Module-doc last paragraph (`:54-55`, "Deliberately absent: the offline path") rewritten: the offline path is a `WriteStore` arm, and the recorder still knows nothing about it. |
| 16 | `crates/htui-store/tests/pg_criteria.rs` | UPDATE | T17 | §11 criteria 3 and 7 after `upload_pending` (F-3). |
| 17 | `crates/htui-agent/src/replay.rs` | CREATE | T18 | `envelope_from_row`, `envelope_or_other`, `envelopes`, `ReplayError` (B.9, E). |
| 18 | `crates/htui-agent/src/lib.rs` | UPDATE | T18 | `pub mod replay;` + `pub use replay::{ReplayError, envelope_from_row, envelope_or_other, envelopes as replay_envelopes};` and the module-doc line "the offline session path" (`:13`) dropped from "deliberately absent". |
| 19 | `crates/htui-agent/tests/replay.rs`, `tests/snapshots/replay__*.snap` | CREATE | T18 | The decode table as a test, the row-level round trip, the fixture snapshots (F-4). |
| 20 | `crates/htui/src/store_worker.rs` | UPDATE | T19, T21 | T19: `StoreRequest::StepEvents`, `StoreReply::StepEvents`, `name()` and `try_serve` arms; the stale "MOD-2 adds `StepEvents(StepId)` here" sentence (`:46`) goes. T21: `StoreReply::ChatAccepted` gains `writer_label` (B.13) — **file-set addition** to T21, no conflict (T19 precedes it). |
| 21 | `crates/htui/src/app/action.rs` | UPDATE | T19 | `Action::Replay { step_id }`. |
| 22 | `crates/htui/src/app/state.rs`, `src/app/mod.rs` | UPDATE | T19 | `App.replay_tab: Option<TabId>` beside `migration_overlay` (`state.rs:172`), set in `register_all` (`app/mod.rs:62` precedent). **File-set addition**: the shell must not name a concrete view (`state.rs:171`), so `App::update` cannot write `ChatTab::ID`. |
| 23 | `crates/htui/src/app/update.rs` | UPDATE | T19 | The `Action::Replay` arm: focus, then dispatch under the Chat tab's origin (B.11). |
| 24 | `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | T19 | `Enter` that did not fold → `self.detail.on_key(key, ctx)` (**file-set addition**, see the preamble). |
| 25 | `crates/htui/src/ui/tabs/backlog/detail/runs.rs` | UPDATE | T19 | Step rows under each run, a step cursor on `J`/`K`, `Enter` emits `Action::Replay` (B.12). |
| 26 | `crates/htui/src/keymap.rs` | UPDATE | T19 | One `KeyScope::Tab(BacklogTab::ID)` binding for `Enter` with help `"replay step"`; its action is the status-line refusal that fires only when no pane consumed the key (B.12 says why the payload cannot live here). |
| 27 | `crates/htui/src/ui/tabs/chat/transcript.rs` | UPDATE | T20 | `Transcript::from_rows(&[SessionEvent])`, `Transcript::len()`. |
| 28 | `crates/htui/src/ui/tabs/chat/mod.rs` | UPDATE | T20, T21 | T20: `ReplayState`, the replay branch of `on_key`, the `StepEvents` reply arm, header/body/hint for replay (B.13). T21: `ChatSessionState.buffered` and the D42 header suffix. |
| 29 | `crates/htui/src/testkit.rs` | UPDATE | T19, T21 | T19: `Harness::with_replay_tab(TabId)`. T21: `Harness::over_backend(Backend)` (`over(store)` becomes a one-line delegate). |
| 30 | `crates/htui/tests/replay.rs`, `tests/snapshots/{backlog__runs_step_selected,chat__replay_*}.snap` | CREATE | T19, T20 | The Runs-pane selection snapshot (T19) and the four replay snapshots (T20), F-5. |
| 31 | `crates/htui/tests/snapshots/backlog__detail_runs.snap` | RE-ACCEPT | T19 | Gains one step line per step under the FEAT-1 run; nothing else moves (`cargo insta review`, line by line). |
| 32 | `crates/htui/src/agent_worker.rs` | UPDATE | T21 | The refusal at `:333-337` and its comment go; `writer_label: writer.label()` captured into `ChatArgs` before the writer moves; `ChatAccepted { .., writer_label }`. |
| 33 | `crates/htui-store/src/testkit.rs`, `crates/htui-store/Cargo.toml`, `crates/htui-store/tests/{cache,connect,migrations,pg_conformance,pg_criteria}.rs` | UPDATE | T21 | `tests/common/mod.rs` promoted to `htui_store::testkit` behind a new `test-support` feature (`demo` + `sqlx` already present), plus `testkit::seed_mirror` (F-6). The five test files replace `mod common;` with `use htui_store::testkit as common;`. **File-set addition**, reason in F-6: criterion 12 needs a driver **and** a throwaway database in one test, and only `htui` depends on both crates. |
| 34 | `crates/htui/Cargo.toml` | UPDATE | T21 | dev-dep `htui-store = { workspace = true, features = ["demo", "test-support"] }`. |
| 35 | `crates/htui/tests/chat_offline.rs`, `tests/snapshots/chat__chat_buffered.snap` | CREATE | T21 | The offline chat end to end (F-6). |
| 36 | `crates/htui-core/src/store/traits.rs` | UPDATE (docs) | T16 | `WriteStore`'s "Only a backend that can reach Postgres implements it" (`:47-48`) and the module doc's "`agent` … not mirrored" reason (`:11-14`) are amended; no signature changes (`pg_conformance.rs` `EXPECTED_CASES` untouched). |
| 37 | `docs/ANA-9.md` §4.4 (`:307-309`), `README.md`, `crates/htui/src/ui/tabs/settings/agents.rs` (comments at `:30`, `:71-73`) | UPDATE (docs) | close-out | "Mirrored: … `agent` (MOD-2 milestone 4)"; README: replay keys and the buffered header. The HANDOFF MOD-4 line (`0003_orchestration.sql`) per the plan's risk row. |

---

## B. Interfaces, exactly

### B.1 `crates/htui-store/cache_migrations/0002_agent_mirror.sql`

Column set = `migrations/0001_init.sql:94-106`; type mapping = the `0001_mirror.sql:5-12` table
(`UUID → TEXT`, `TIMESTAMPTZ → INTEGER` µs, `TEXT[] → TEXT` JSON array, `JSONB → TEXT`,
`BOOLEAN → INTEGER`). As in `0001_mirror.sql`: no `CHECK`, no `UNIQUE` (`workspace.slug` has none
in the mirror either), no foreign key, no index (two rows ordered by name need none, and
`idx_cache_*` indexes exist only where `cache/read.rs` needs one, `:21-22`).

```sql
-- ------------------------------------------------------------------------------------------------
-- The `agent` registry row, mirrored (MOD-2 milestone 4, plan D31 / D32; `docs/ANA-9.md` 4.4 amended).
--
-- Same columns as `migrations/0001_init.sql` `agent`; the `0001_mirror.sql` type mapping. Unscoped:
-- a full replace on every pass beside `app_user`, `workspace` and `workspace_project`, so no
-- `cache_cursor` row ever names it.
--
-- `agent_box` is deliberately absent: it is a probe snapshot whose columns milestone 5's
-- `0002_agent_probe.sql` changes, and offline "which box" is this box (D31). The `0001_mirror.sql`
-- header's "not mirrored: agents" is superseded here, not edited there (forward-only).
-- ------------------------------------------------------------------------------------------------

CREATE TABLE agent (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, transport TEXT NOT NULL,
    launch TEXT NOT NULL, models TEXT NOT NULL DEFAULT '[]', default_model TEXT,
    billing TEXT NOT NULL, enabled INTEGER NOT NULL DEFAULT 1,
    settings TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
```

### B.2 `crates/htui-store/src/cache/refresh.rs`

```rust
/// `agent` (§5.7), unscoped: mirrored whole on every pass (D32).
const AGENT_COLUMNS: &[&str] = &[
    "id", "name", "transport", "launch", "models", "default_model", "billing", "enabled",
    "settings", "created_at", "updated_at",
];

/// Step 1, fourth table: `DELETE FROM agent`, then one upsert per server row — the
/// `replace_app_user` shape (`:399-429`) with `json_text` for `launch` / `settings`,
/// `strings_text` for `models`, `i64::from(enabled)`, `ts_bind` for the two stamps.
/// `report.add("agent", rows.len())`.
async fn replace_agent(pool: &PgPool, sqlite: &SqlitePool, report: &mut PassReport) -> Result<()>;
```

`sqlx::query!("SELECT id, name, transport, launch, models, default_model, billing, enabled,
settings, created_at, updated_at FROM agent")` — `transport` / `billing` arrive as `String` (the
columns are `TEXT` with a `CHECK`, not enum-typed), bound verbatim. Called at `run_pass` right after
`replace_app_user` (`:243`) so the pass order matches `MIRRORED_TABLES` order.

### B.3 `crates/htui-store/src/cache/read.rs` and `identity.rs`

```rust
// identity.rs
/// `app_user.name` of this OS user: `USERNAME`, then `USER`, then `htui` (ANA-9 §5.10). The one
/// definition both `PgStore::seed_if_empty` and `CacheStore::this_user` derive from (D33).
#[must_use] pub fn os_user_name() -> String;

// read.rs, inside the "four inherent reads" impl block (now six)
impl CacheStore {
    /// The mirrored registry, ordered by `agent.name`, every row `on_box: None` (D31): the mirror
    /// holds no `agent_box`, and offline "this box" is the only box there is.
    /// # Errors  driver errors through `map_sqlx`; `StoreError::Backend` from a decoder on a corrupt mirror.
    pub async fn agents(&self) -> Result<Vec<AgentSummary>>;

    /// `user_named(&identity::os_user_name())` (D33).
    pub async fn this_user(&self) -> Result<UserId>;

    /// The mirrored `app_user` row with this `name`, earliest `created_at` then smallest `id`
    /// (the `MemStore::this_user` tiebreak, `mem.rs:115-125`).
    /// # Errors  `StoreError::NotFound { entity: "app_user", id: name }` when this box has never
    /// synced such a row — the honest answer, never an invented author for `run.started_by`.
    pub async fn user_named(&self, name: &str) -> Result<UserId>;
}
```

`agents()` SQL: `SELECT id, name, transport, launch, models, default_model, billing, enabled,
settings, created_at, updated_at FROM agent ORDER BY name`. Decode, column by column:
`uuid_col::<AgentId>("agent.id")`, `text("name")`, `get::<Transport>("transport")` and
`get::<Billing>("billing")` (the `str_enum!` sqlx derive, as `get::<EventKind>` at `:572`),
`json_col("agent.launch")`, `strings_col("agent.models")`, `opt_text("default_model")`,
`bool_col(get::<i64>("enabled"))`, `json_col("agent.settings")`, `ts_col("agent.created_at")`,
`ts_col("agent.updated_at")`. `user_named` SQL: `SELECT id FROM app_user WHERE name = ? ORDER BY
created_at, id LIMIT 1`.

### B.4 `crates/htui-store/src/backend.rs`

```rust
impl Backend {
    /// Memory → `Some(Writer::Memory)`, Online → `Some(Writer::Online)`,
    /// Offline → `Some(Writer::Buffered(BufferedWriter::new(cache.clone())))` (D34).
    /// Stays `Option`: `writable()` is the paired API and stays `Option`, and a fourth variant
    /// that cannot write is a one-arm change here rather than a signature change everywhere.
    #[must_use] pub fn writer(&self) -> Option<Writer>;
    /// Offline → `cache.this_user().await` (D33): `NotFound` on an unsynced mirror.
    pub async fn this_user(&self) -> Result<UserId>;
    /// Offline → `cache.agents().await` (D31): `on_box: None` on every row.
    pub async fn agents(&self) -> Result<Vec<AgentSummary>>;
    /// **Unchanged**: `false` for `Offline`. It no longer means "no writer"; it means "the server
    /// is not reachable", which is what the re-dial ticker keys on
    /// (`store_worker.rs:498`: `reconnect.is_some() && !backend.is_writable() && held.is_none()`).
    /// Answering `true` would stop the shell from ever dialling again.
    #[must_use] pub const fn is_writable(&self) -> bool;
    /// **Unchanged**: `None` for `Memory` and `Offline` (a borrowed `&PgStore` is the server or nothing).
    #[must_use] pub const fn writable(&self) -> Option<&PgStore>;
}
```

Module doc (`:6-12`), amended text: *"There are two ways to reach a `WriteStore`: `writable()`
borrows a `PgStore` for one call and answers `None` unless the server is reachable; `writer()`
hands out an owned `Writer` on every variant — since milestone 4 the `Offline` one is
`Writer::Buffered`, which reaches a file under `<cache_dir>/pending/` and never the server. The
invariant that survives is narrower and still true: nothing writes to Postgres unless `Online`."*

The `Settings` tab consequence is stated in `agents()`'s new doc: offline it now lists the mirrored
rows with every `on_box` = `None`, which the tab already renders as "not probed"
(`crates/htui-core/src/model/agent.rs:157`).

### B.5 `crates/htui-store/src/cache/pending.rs` `[H-1]`

```rust
/// Suffix of a buffer a chat is still writing: `<project>.<run>.jsonl.open`. `upload_pending`'s
/// `list()` filters on the `jsonl` extension (`:186`), so an open buffer is invisible to it by
/// construction rather than by a check it could forget.
pub const OPEN_SUFFIX: &str = "open";

/// As today, but the file it appends to is the **open** name. Signature unchanged.
pub async fn append_pending(dir: &Path, project: ProjectId, run: RunId, events: &[SessionEvent]) -> Result<usize>;

/// Renames `<project>.<run>.jsonl.open` to `<project>.<run>.jsonl`, making it uploadable.
/// `Ok(false)` when no open buffer exists (a chat that recorded nothing, or one already sealed).
/// # Errors  `StoreError::Backend` with the path when the rename fails.
pub async fn seal_pending(dir: &Path, project: ProjectId, run: RunId) -> Result<bool>;

/// Seals every `pending/*.jsonl.open`: called by `CacheStore::open`, when no chat of this process
/// can be live, so an open buffer found there is a crashed chat whose rows are complete as far as
/// they got (`R-HIS-1` for the crash case). Returns how many were sealed.
/// # Errors  `StoreError::Backend` when `pending/` exists but cannot be listed.
pub fn seal_orphaned(dir: &Path) -> Result<usize>;
```

Module doc addition, under the naming-contract paragraph: *"A buffer is written under the `.open`
suffix while its chat is live and sealed by `seal_pending` when the chat ends; `upload_pending`
reads sealed files only, so a chat that outlives a reconnect is uploaded once, whole, on the pass
after it ends."* (Known limit, stated there too: a second `htui` process on the same box seals the
first one's live buffer at start; the rows still all land, on two passes, and the first pass's
`run_step.usage` is partial — narrower than the race without the suffix, and the ANA-9 §4.4
two-process case is "just another reader".)

### B.6 `crates/htui-store/src/writer.rs`

```rust
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A writable store a caller can hold.
#[derive(Debug, Clone)]
pub enum Writer {
    Memory(MemStore),
    Online(PgStore),
    /// The offline sink (D34): reads from the mirror, writes to `<cache_dir>/pending/`.
    Buffered(BufferedWriter),
}
impl Writer {
    /// `"memory"` | `"online"` | `"buffered"` (D42: the chat header states the last one).
    #[must_use] pub const fn label(&self) -> &'static str;
}

/// The offline `WriteStore` (D34, D35): the same rows the online writer takes, appended as JSON
/// lines through `append_pending`. Holds no lock across an `.await`: the registration map is a
/// `std::sync::Mutex` read into a local before the file write is awaited.
#[derive(Debug, Clone)]
pub struct BufferedWriter {
    /// Reads, and `dir()`.
    cache: CacheStore,
    /// `cache.dir()`, copied once: the argument every `pending::*` call takes.
    dir: PathBuf,
    /// `start_chat_run` fills it; `append_events` reads it. `Arc` so a clone of the writer (the
    /// recorder borrows one; `ChatArgs` owns one) shares the registrations.
    runs: Arc<Mutex<HashMap<StepId, (ProjectId, RunId)>>>,
}
impl BufferedWriter {
    #[must_use] pub fn new(cache: CacheStore) -> Self;
    /// `<root>/cache/<fingerprint>` — the buffer lives at `dir().join("pending")`.
    #[must_use] pub fn dir(&self) -> &Path;
    /// The `(project, run)` a step was registered under, for tests and logs.
    #[must_use] pub fn run_of(&self, step: StepId) -> Option<(ProjectId, RunId)>;
}
```

`impl ReadStore for BufferedWriter` — the seven methods (`traits.rs:30-45`) delegate to
`self.cache` verbatim (the recorder never reads; the trait bound `WriteStore: ReadStore` wants them).

`impl WriteStore for BufferedWriter` — every method, per D35, exactly:

| Method | Answer | Why |
|---|---|---|
| `mint_item` | `Err(StoreError::Unreachable("item writes need the server; offline item editing is MOD-13's"))` | D35 |
| `update_item` | same `Unreachable` | D35 |
| `transition` | same `Unreachable` | D35 |
| `upsert_agent` | `Err(StoreError::Unreachable("the agent registry is written on the server only"))` | D35 |
| `upsert_agent_box` | same as `upsert_agent` | D35 |
| `append_events(events)` | empty → `Ok(0)`. Otherwise: group by `run_step_id` keeping first-seen order; **before any write** resolve every group through `runs` (lock taken and dropped inside a block) — a missing step → `Err(StoreError::NotFound { entity: "run_step", id })` and **nothing is written** (the trait's "either every new row lands or none", `:66-68`); then one `append_pending(&dir, project, run, group)` per group, summing the returned counts. | plan T16: "an event for an unregistered step is a `NotFound`, not a silent drop" |
| `set_step_usage` | `Ok(())` + `tracing::debug!(%step, "buffered: run_step.usage is recomputed at upload (D36)")` | D35 — the line format holds `session_event` columns only |
| `start_chat_run(chat)` | `runs.insert(chat.step_id, (chat.project_id, chat.run_id))`; `Ok(())`. Writes no row: the `run` / `run_step` pair is synthesised by `upload_one` from the file name and the events. | D34 |
| `finish_chat_run(run, step, ..)` | `[H-1]` `seal_pending(&dir, project, run).await` where `(project, run)` = `runs[step]` (missing → `NotFound`, as above); the registration is **kept** (a late flush after the seal appends to a new `.open` file rather than losing rows; it is sealed at the next start or by a later seal). Without H-1: `Ok(())` + a debug line. | D35 as amended by H-1 |

Two consequences stated in the type's doc: (1) an offline chat is **not** in the mirror's `run`
table, so `active_runs` and the top bar do not count it — the D42 header is the indicator; (2) every
`Backend::writer()` call builds a fresh, empty `BufferedWriter`, which is right because the runtime
takes exactly one per `ChatStart` and moves it into `ChatArgs` for the chat's life
(`agent_worker.rs:335`, `:417`), so `start_chat_run` and every `append_events` see one map.

### B.7 `crates/htui-core/src/model/usage.rs`

```rust
//! `run_step.usage` (ANA-9 §4.3, ANA-4 §7): the one summing rule (D36). The token fields of a
//! `usage` row are per-row **deltas** summed with saturating adds; a key no row ever carried stays
//! `null`, so "reports no tokens" and "reported zero" stay distinct.

/// The five §4.3 keys, each nullable. Serde form == the `run_step.usage` document, key order as
/// declared (identical to the `json!` the recorder wrote before the move).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageTotals {
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub cost_micros: Option<i64>,
}
impl UsageTotals {
    /// Adds one `usage` **payload**'s five keys (integers only; a key that is absent, `null` or not
    /// an integer adds nothing). Takes the JSON document, not a typed event, because the typed
    /// event lives in `htui-agent` and the uploader in `htui-store` — both hold the payload.
    pub fn add_payload(&mut self, payload: &Value);
    /// `add_payload` over every row with `kind == EventKind::Usage`, in slice order; other kinds
    /// are skipped. The uploader's entry point.
    #[must_use] pub fn from_rows(rows: &[SessionEvent]) -> Self;
    /// `serde_json::to_value(self)` — the document `set_step_usage` and `upload_one` write.
    #[must_use] pub fn to_value(self) -> Value;
}
```

Recorder side (`record.rs`): `Recorder.usage: UsageTotals`; `self.usage.add(usage)` at `:593-596`
becomes `self.usage.add_payload(&payload)` on the same branch (the scrubbed document that is about
to be persisted — the same bytes the uploader will read), and `record_unreadable` (`:750-771`) adds
the payload too when `kind == EventKind::Usage`, so "every persisted `usage` row is summed" holds on
both sides by construction rather than by coincidence. `RecorderSummary.usage` stays a `Value`
(`self.usage.to_value()`). `tests/recorder.rs` is the proof the move changed nothing
(`usage_deltas_sum_to_step_usage` in `conformance.rs` too).

### B.8 `crates/htui-store/src/cache/pending.rs` — `upload_one` (D36)

Per step, before its insert: `digest` = the `payload["digest"]` string of the step's `prompt` row if
any (the recorder wrote it, `record.rs:389`); `usage` = `UsageTotals::from_rows(step_rows).to_value()`
— **always** written, even as five `null`s, because the online recorder writes the same document at
the prompt (`sync_step` fires on `digest_pending`, `record.rs:722-731`) and an uploaded step must be
indistinguishable from an online one. The insert becomes:

```sql
INSERT INTO run_step (id, run_id, position, attempt, fanout_index, phase_name, status,
                      started_at, finished_at, prompt_digest, usage)
VALUES ($1, $2, $3, 1, 0, 'chat', 'done', $4, $5, $6, $7)
ON CONFLICT (id) DO NOTHING
```

`$6: Option<&str>`, `$7: serde_json::Value` (JSONB). Idempotency unchanged: a second pass conflicts
on `id` and inserts nothing. `.sqlx/` regenerated. `run_step.agent_id` / `model` stay `NULL` (H-3).

### B.9 `crates/htui-agent/src/replay.rs`

```rust
//! The inverse of `record.rs` (D37): a persisted `session_event` row back into the
//! `DriverEnvelope` the live path renders. `record.rs` writes each payload as the serde form of
//! the event's inner struct (`round_trip`, `:965-975`), so the inverse is `serde_json::from_value`
//! per kind — a table, not a parser. Section E is that table.

/// A row whose payload does not read as the struct `record.rs` wrote for its kind.
/// Carries the kind, the `seq` and serde's reason; never the payload (it may be masked text).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("seq {seq}: a `{kind}` payload does not decode: {reason}")]
pub struct ReplayError { pub kind: EventKind, pub seq: i32, pub reason: String }

/// The strict decode: `Ok` for every row `record.rs` can have written, `Err` for one it cannot.
/// The three `htui`-authored kinds and `other` are `Ok(Other { .. })` (E). Tests and the
/// regression net call this; the transcript does not.
/// # Errors  `ReplayError` when the payload does not deserialise as the kind's struct.
pub fn envelope_from_row(row: &SessionEvent) -> Result<DriverEnvelope, ReplayError>;

/// D37's policy: `envelope_from_row`, and on `Err` the row as `Other { update: kind.as_str(),
/// body: payload }` — the transcript renders a dim `<kind>` line and loses nothing else.
#[must_use] pub fn envelope_or_other(row: &SessionEvent) -> DriverEnvelope;

/// `envelope_or_other` over rows sorted by `seq` (defensive: every backend already orders, but
/// a slice handed in by a test may not). What `Transcript::from_rows` consumes.
#[must_use] pub fn envelopes(rows: &[SessionEvent]) -> Vec<DriverEnvelope>;
```

Both forms set `raw: row.raw.clone()` and `at: row.at`; `role` and `turn` are not read (the
transcript renders neither). The "column fills the payload" rule and the `message_id` stamp are in E.

### B.10 `crates/htui/src/store_worker.rs`

```rust
pub enum StoreRequest {
    /* existing */
    /// The replay log of one step (D38, `R-HIS-2`). Answered once; replay is a read, not a stream.
    StepEvents(StepId),
}
// name(): "step_events"

pub enum StoreReply {
    /* existing */
    /// Answer to `StoreRequest::StepEvents`. `None` is `ReadStore::step_events`'s "not cached"
    /// (the mirror's last-N window, `cache/read.rs:560-564`) and renders as "this step is not on
    /// this box"; `Some(vec![])` is "this step recorded nothing" (D38).
    StepEvents { step_id: StepId, events: Option<Vec<SessionEvent>> },
    /// T21: gains `writer_label: &'static str` — `Writer::label()` of the store the chat records
    /// into (`"memory"` | `"online"` | `"buffered"`), for D42.
    ChatAccepted { step_id: StepId, session_ref: Option<AgentSessionRef>, caps: DriverCaps, writer_label: &'static str },
}
```

`try_serve` arm: `StoreRequest::StepEvents(step) => StoreReply::StepEvents { step_id: *step, events:
backend.step_events(*step).await? }` — `Backend: ReadStore` already dispatches all three variants
(`backend.rs:330-336`). Served through the ordinary path (it is a read; an `Unreachable` from it
drops `Online` to the mirror like any other read, `:448-458`).

### B.11 `crates/htui/src/app/{action,state,update}.rs`

```rust
// action.rs
pub enum Action {
    /* existing */
    /// Reopen a past step read-only in the Chat tab (D39, `R-HIS-2`). Emitted by the Runs pane.
    Replay { step_id: StepId },
}

// state.rs, beside `migration_overlay` (:172); set by `register_all` (app/mod.rs:62 precedent)
pub struct App { /* existing */ pub replay_tab: Option<TabId> }

// update.rs
impl App {
    // Action::Replay { step_id } => self.replay(step_id)
    /// `replay_tab == None` → `status = Some("no tab can replay a step")` and nothing else.
    /// Otherwise `update_tab(TabAction::Focus(tab))` (the existing activation path, which issues
    /// the Chat tab's `wants_requests` — `Agents`, harmless), then
    /// `dispatch(Origin::Tab(tab), StoreRequest::StepEvents(step_id))`. The reply is addressed to
    /// the Chat tab and gated by `is_fresh` under `(Tab(chat), discriminant(StepEvents))`: a second
    /// replay supersedes the first, exactly the `latest` rule every read has (`state.rs:261-275`).
    fn replay(&mut self, step_id: StepId);
}
```

### B.12 `crates/htui/src/ui/tabs/backlog/detail/runs.rs`, `backlog/mod.rs`, `keymap.rs`

```rust
pub struct RunsTab {
    runs: Vec<RunSummary>, item: Option<ItemId>, scroll: Scroll,
    /// Index into the flattened `(run, step)` list, newest run first, steps in reply order
    /// (`RunSummary.steps` is already ordered by `(position, attempt, fanout_index)`,
    /// `run.rs:319`). `None` when no run has a step.
    selected: Option<usize>,
}
impl RunsTab {
    /// The selected step, for tests.
    #[must_use] pub fn selected_step(&self) -> Option<StepId>;
}
```

`DetailTab::on_key` for `RunsTab`: `J` / `K` move `selected` (clamped; the table skips whole runs
above the selected one so the cursor stays visible — the `Scroll` offset becomes derived, not
keyed); `PageDown` / `PageUp` keep `Scroll::on_key`; `Enter` with `Some(step)` →
`ctx.emit(Action::Replay { step_id })`, `Consumed`; anything else `Pass`. `on_reply(Runs)` resets
`selected` to the first step when any exists. Render: under each two-line run row, one one-line
`Row` per step — `  ▸ <position> <phase_name> <status> <started STAMP>` — accent on the selected
step, `theme.dim` on the rest; the `▸` is a space on unselected rows so column widths do not move.

`backlog/mod.rs:207`: `KeyCode::Enter => return match self.fold() { Handled::Consumed =>
Handled::Consumed, Handled::Pass => self.detail.on_key(key, ctx) }` — a project header still folds;
an item row's `Enter` reaches the active pane.

`keymap.rs` — one binding in `default_global()`: `Binding { scope: KeyScope::Tab(BacklogTab::ID),
key: Enter, action: Action::Error("select a step in the Runs pane (J/K) to replay it"),
help: "replay step" }`. The *payload* (which step) cannot live in a static binding; the pane emits
`Action::Replay` and consumes the key, so this row fires only when no pane took `Enter` — and then
the status line says what to do. The help line (`state.rs:480`, `help_line(&KeyScope::Tab(id))`)
gains `Enter replay step` on the Backlog tab.

### B.13 `crates/htui/src/ui/tabs/chat/{transcript,mod}.rs`

```rust
// transcript.rs
impl Transcript {
    /// `Self::new()` then `apply` over `htui_agent::replay::envelopes(rows)`: one code path for
    /// live and replay (D37, `R-TUI-6`). The banner row sets `session_ref` exactly as it does live.
    #[must_use] pub fn from_rows(rows: &[SessionEvent]) -> Self;
    /// Row count, for the replay header.
    #[must_use] pub fn len(&self) -> usize;
    #[must_use] pub fn is_empty(&self) -> bool;
}

// mod.rs
/// A past step open read-only (D40). While `Some`, the tab sends nothing.
#[derive(Debug)]
pub struct ReplayState {
    pub step_id: StepId,
    pub transcript: Transcript,
    /// The reply was `None`: the step is not on this box (D38).
    pub missing: bool,
}
pub struct ChatSessionState { /* existing */ /// `writer_label == "buffered"` (D42).  pub buffered: bool }
pub struct ChatTab { /* existing */ replay: Option<ReplayState> }
impl ChatTab {
    /// The open replay, for assertions.
    #[must_use] pub const fn replay(&self) -> Option<&ReplayState>;
}
```

`on_key`, first thing after clearing `refusal`, **before** the composer is consulted:

| key (replay `Some`) | effect |
|---|---|
| `Esc` | `self.replay = None`; `Consumed` (the live session, transcript and composer are exactly as they were) |
| `j` `k` `g` `G` `t` | `replay.transcript.on_key(key)` |
| `1`..`9`, `i`, `Enter`, `a` | `Consumed`, nothing else (D40, D41: digits answer nothing, the composer never opens) |
| anything else | `Pass` |

The branch contains no `ctx.request` and no `ctx.emit` — the property a reviewer greps for (D40).
`on_reply`: `StepEvents { step_id, events }` → `self.replay = Some(ReplayState { step_id,
transcript: Transcript::from_rows(events.as_deref().unwrap_or(&[])), missing: events.is_none() })`;
`ChatAccepted { writer_label, .. }` → `buffered: writer_label == "buffered"`. Live `Chat(..)`
frames keep applying to `self.transcript` while a replay is open.

Render in replay: header `replay · step <first 8 hex of id> · <n> rows` (accent) `· read-only ·
Esc leave` (dim); no caps banner; body = `replay.transcript.lines(..)`, or the dim one-liner
`this step is not on this box` when `missing`, or `this step recorded nothing` when the transcript
is empty; no permission strip (a parked-looking `Permission` row renders inside the transcript as
history, D41, and `PermissionStrip::render` is not called); the last line is a plain dim hint
`Esc leave replay · t thoughts · j/k scroll` in place of `Composer::render`. Live header gains
` · buffered · uploads when the store returns` (dim) when `session.buffered` (D42).

### B.14 `crates/htui/src/agent_worker.rs` and `testkit.rs`

`start()` (`:333-337`): the refusal and its two-line comment are deleted; `backend.writer()`'s
`ok_or_else` stays as the defensive arm with a new message (`"this backend hands out no writer"`)
because the signature is still `Option`. `ChatArgs` gains `writer_label: &'static str`
(= `writer.label()` taken before the move); `run_chat` passes it in `ChatAccepted`.

```rust
impl Harness {
    /// A shell over any backend — the offline snapshot needs `Backend::Offline` over a seeded mirror.
    #[must_use] pub fn over_backend(backend: Backend) -> Self;   // `over(store)` = `over_backend(Backend::memory(store))`
    /// `App.replay_tab`, as `register_all` sets it.
    #[must_use] pub fn with_replay_tab(mut self, tab: TabId) -> Self;
}
```

---

## C. Data flow

### C-1. Offline chat → recorder → `BufferedWriter` → jsonl → refresher → `upload_pending` → Postgres

```
ChatTab ──ChatStart──▶ store_worker ──▶ AgentRuntime::serve ──▶ start()
                                          │ backend = Offline { cache }
                                          ├ writer()   = Writer::Buffered(BufferedWriter{cache,dir,runs})
                                          ├ box_info() = cache.box (own row)            [existing]
                                          ├ this_user()= cache.app_user WHERE name = os_user_name()   (D33)
                                          ├ agents()   = cache.agent, on_box: None      (D31)
                                          ├ driver_for(agent, None) → AcpDriver / fake
                                          └ writer.start_chat_run(chat) → runs[step] = (project, run)
run_chat ── driver.start ── ChatAccepted{writer_label:"buffered"} ──▶ header "buffered · uploads when the store returns" (D42)
   │  Recorder<'_, Writer>::record / record_prompt / … (scrub → buffer → flush)
   │     flush → writer.append_events(rows) → group by step → runs[step] → append_pending(dir, project, run, rows)
   │                                                                  └▶ <dir>/pending/<project>.<run>.jsonl.open  (+n lines, seq order)
   │     sync_step → writer.set_step_usage → Ok(()) (debug)                                          (D35)
   └  finish → close_run → writer.finish_chat_run → seal_pending → <project>.<run>.jsonl            [H-1]
… later, ConnEvent::Online → go_online → Refresher::spawn → run_pass
   step 1: replace_app_user, replace_agent, replace_workspace, …                                     (D32)
   step 5: upload_pending(pool, cache.dir(), this_box, this_user)
             parse(<project>.<run>.jsonl) → upload_one: INSERT run (done) · INSERT run_step (prompt_digest, usage = UsageTotals::from_rows) · INSERT session_event × n · commit · delete file   (D36)
```

Prose, the points that are not obvious from the diagram:

1. Nothing in the recorder changes for the offline path (D34): it is generic over `S: WriteStore`,
   and `Writer::Buffered` is one more arm of the enum it already takes. Every conformance case that
   passes online passes offline with the same rows.
2. The user the offline chat names (`ChatRunSpec::mint(.., user, ..)`) is discarded by the buffer —
   the line format has no `started_by` — and re-supplied at upload from `RefreshSettings.this_user`
   (= `PgStore::this_user()` at connect, `refresh.rs:52-55`). D33 derives the offline one by the same
   `os_user_name()` the online seed used, so the two are the same row; the offline lookup exists so
   `run.started_by` can be *checked* before the chat starts (an unsynced mirror refuses, `NotFound`)
   rather than to be persisted. Same for `target_box_id`: `box_info()` offline reads the own row the
   refresher mirrored, and `upload_one` writes `settings.this_box`.
3. A mirror that has never synced (first launch, offline) refuses at `box_info()` — "this box is not
   registered", the milestone-3 message — before `this_user()` is reached.
4. The offline chat is in no `run` table until upload: `active_runs` does not count it and the Runs
   pane cannot list it (a free-standing chat has `item_id NULL` anyway, `recorder.rs` doc). The
   header is the one place it is visible (D42).
5. `finish_chat_run` seals; `upload_pending` reads sealed files only; the chat that straddles the
   reconnect is uploaded once, whole, on the first pass after it ends `[H-1]`.

### C-2. Runs pane step → `Action::Replay` → dispatch under the Chat tab's origin → `StepEvents` → decode → `Transcript`

```
BacklogTab::on_key(Enter) ─fold()=Pass─▶ DetailRegistry::on_key ─▶ RunsTab::on_key
                                                                    └ selected step → ctx.emit(Action::Replay{step_id})
App::drain(Origin::Tab(backlog)) → App::update(Action::Replay)
   ├ update_tab(TabAction::Focus(replay_tab))          (Chat tab active; its wants_requests = [Agents])
   └ dispatch(Origin::Tab(chat), StoreRequest::StepEvents(step))   latest[(Tab(chat), StepEvents)] = seq
store_worker: try_serve → backend.step_events(step)  (Memory | Online: pg | Offline: cache, None = not cached)
   → ReplyEnvelope{seq, origin: Tab(chat), StoreReply::StepEvents{step_id, events}}
event_loop → App::update(Action::Reply) → is_fresh(Tab(chat), seq) → tabs.by_id_mut(chat).on_reply
   ChatTab::on_reply → replay::envelopes(rows) → Transcript::from_rows → ReplayState{step_id, transcript, missing}
render: header "replay · step … · read-only · Esc leave" ▸ transcript.lines() (same renderer as live) ▸ hint
keys: Esc → replay = None (live state untouched); digits/i/Enter/a consumed, no request (D40)
```

Prose: the reply is addressed by origin, not by which tab is active, so it lands in the Chat tab
even if the user tabbed away before it arrived (`update.rs:120-147`). Decoding runs in `on_reply`
on the UI task — it is `serde_json::from_value` over the rows the reply already carries, no store
handle and no I/O (`R-NF-3`); a mirror step is at most `cached_transcript_steps` deep and a 5 000-row
step (§11.4's size) decodes in milliseconds. If that ever changes, the decode moves into
`try_serve` behind the same reply type.

---

## D. Ownership and locks

| Thing | Owner | Lifetime |
|---|---|---|
| `BufferedWriter` (and its `Arc<Mutex<HashMap>>`) | `ChatArgs` → `run_chat`'s stack frame; the `Recorder` borrows it | the chat |
| `CacheStore` handle inside it | clone of the backend's (`Arc` pool) | the chat |
| `<project>.<run>.jsonl.open` | the filesystem; written only through `append_pending` from `run_chat`'s task | until `seal_pending` |
| `<project>.<run>.jsonl` | the filesystem; read and deleted only by `upload_pending` in the refresher task | until the commit after which it is deleted |
| `ReplayState` and its `Transcript` | `ChatTab` | until `Esc` or the next `StepEvents` reply |
| `Vec<SessionEvent>` of a `StepEvents` reply | moves through `ReplyEnvelope` into `on_reply`, decoded, dropped | one `update` |

Locks: `BufferedWriter.runs` is a `std::sync::Mutex` taken in a block that copies the `(project,
run)` pairs out and ends before `append_pending(..).await` — the crate rule. No new channel; the
reply channel and the request channel are the existing unbounded pair. `upload_pending` and a live
chat never touch the same file `[H-1]`.

---

## E. The decode table (`replay::envelope_from_row`)

Input: one `SessionEvent` (`event.rs:59-78`). `payload` is what `record.rs` wrote: for the eight
driver kinds the serde form of the inner struct (`round_trip`, `:965-975`; `assistant_text` /
`thought` are `json!({ "text" })` only, `:551`); for the three `htui` kinds the `json!` at `:379`
(`prompt`: `text`, `sections`, `digest`), `:427` (`follow_up`: `text`), `:465-470`
(`permission_answer`: `request_id`, `option_id`, `by`, `cancelled`).

**Column fill.** For `tool_call`, `tool_result`, `edit_proposal`, `permission_request`: when
`payload` is an object **without** a `tool_call_id` key and `row.tool_call_id` is `Some`, the
column's value is inserted before decoding. The recorder writes both from one value, so a recorder
row never needs it; ANA-9 §4.3's own key list omits the id (it is the column), and the demo fixture
follows §4.3 (`fixtures.rs:1250-1270`), so without the fill the fixture's `tool_call` would not
decode. When the key is present the payload wins.

**`message_id` stamp.** `assistant_text` / `thought` rows carry no `message_id`. The live tab
opens one `Assistant` row per `message_id` group (`transcript.rs:331-349`) and the recorder writes
one row per group (`record.rs:532-538`); decoding two adjacent rows with `message_id: None` would
make `append_text` join them into one row and glue the last line of one to the first line of the
next. So the decoder stamps `message_id = Some(format!("seq:{}", row.seq))`: one persisted row, one
transcript row, which is the live structure. (The only divergence left is a run the 16 KiB bound
split, `CHUNK_FLUSH_BYTES`, which live rendered as one row — accepted, stated in the doc.)

| `EventKind` | `DriverEvent` | Payload → struct | Notes |
|---|---|---|---|
| `Prompt` | `Other { update: "prompt", body: payload }` | none | **No `DriverEvent` variant exists** — `DriverEvent` is `EventKind` minus the three `htui`-authored kinds by design (`event.rs:155-159`). Live, `Frames::local("prompt", { text })` ships the same shape (`agent_worker.rs:596`), and `Transcript::apply_other` reads `body["text"]` only (`transcript.rs:276-285`); the replay body is a superset (`sections`, `digest`) and renders identically. |
| `FollowUp` | `Other { update: "follow_up", body: payload }` | none | same reason; body `{ text }` is byte-identical to the live frame (`follow_up_frame`). |
| `PermissionAnswer` | `Other { update: "permission_answer", body: payload }` | none | same reason; `apply_other` → `resolve_permission(body)` marks the matching `Permission` row resolved (`:287`, `:304-328`) — D41's "renders resolved" is this existing arm, untouched. |
| `AssistantText` | `AssistantChunk(TextChunk)` | `from_value::<TextChunk>` (`message_id` defaults to `None`, then stamped `seq:N`) | one row → one `Assistant` transcript row |
| `Thought` | `ThoughtChunk(TextChunk)` | same | folded by `t` as live |
| `ToolCall` | `ToolCall(ToolCallEvent)` | column fill → `from_value::<ToolCallEvent>`; `locations` defaults empty | `input` is verbatim JSON, any shape |
| `ToolResult` | `ToolResult(ToolResultEvent)` | column fill → `from_value`; `output`, `locations`, `terminal_reason` default | `Transcript::apply` folds it into its `ToolCall` row (`:206-219`), including the synthesized `rejected` / `cancelled` results |
| `EditProposal` | `EditProposal(EditProposalEvent)` | column fill (optional id) → `from_value`; `accepted` missing → `None` (serde's `Option` default) | dedup by `(id, path)` happens in `apply` as live (`:220-239`) |
| `PermissionRequest` | `PermissionRequest(PermissionRequestEvent)` | column fill (optional) → `from_value`; `PermissionRequestId` is `#[serde(transparent)]` (`driver.rs:76-78`) | a request never answered stays `resolved: None` — parked-looking history, not answerable (D41: replay consumes digits and calls `PermissionStrip` never) |
| `Plan` | `Plan(PlanEvent)` | `from_value::<PlanEvent>` | an unknown `status` / `priority` string is a serde error → strict `Err`, lossy `Other { update: "plan" }` |
| `Usage` | `Usage(UsageEvent)` | `from_value::<UsageEvent>` (`#[serde(default)]` on the struct: any subset of keys decodes; the fixture's `cost_micros: null` too) | rendered by `usage_line` as live |
| `Error` | `Error(ErrorEvent)` | `from_value::<ErrorEvent>` (`code`, `message` required) | includes the recorder's own `scrub_residue` rows (role `htui`) |
| `Done` | `Done(DoneEvent)` | `from_value::<DoneEvent>` | unknown `stop_reason` → `Err` / `Other { update: "done" }` |
| `Other` | `Other(OtherEvent)` | `from_value::<OtherEvent>` — the persisted payload **is** `{ update, body }` (`round_trip` over `OtherEvent`) | the `session_started` banner comes back as `Other { update: "session_started", body }` and `apply_other` lifts `body["session_id"]` into `session_ref` for the header (`:290-296`); a payload without `update` (a hand-written row) → `Other { update: "other", body: payload }` |
| any kind, payload not an object or not decodable | strict: `Err(ReplayError { kind, seq, reason })`; lossy: `Other { update: kind.as_str(), body: payload }` | | D37: "never an error that costs the transcript" — the transcript uses the lossy form and shows a dim `<kind>` line |

Which kinds become `DriverEvent::Other`, summarised: `prompt`, `follow_up`, `permission_answer`
(no variant exists and the live UI already receives them as `Other`), `other` (identity, including
the banner), and any row of any kind whose payload the lossy form could not decode.

---

## F. Test seams

### F-1. T15 — `crates/htui-store/tests/cache.rs` (Postgres-gated, `demo_db()`)

- `a_pass_mirrors_the_agent_registry`: `run_pass` → `report.rows("agent") == 2`;
  `cache.agents().await` equals `MemStore::demo().agents().await` with every `on_box` set to `None`,
  ordered `agy`, `claude`; a second pass leaves the count at 2 (replace, not accumulate).
  `a_pass_mirrors_the_demo_projects` (`:157`) asserts per-table counts and `report.uploaded`, not an
  exact `report.tables` list, so it gains one line: `assert_eq!(report.rows("agent"), 2)`.
- `an_offline_backend_lists_the_mirrored_registry`: `Backend::Offline { cache, since: Some(now) }` →
  `agents()` is `Ok` with the two names; `backend.rs`'s doc example.
- `this_user_resolves_the_synced_name_and_refuses_an_unsynced_one`: `cache.user_named("luigi")` ==
  `ids::USER` (the fixture user's name, `fixtures.rs:352`, read as `demo_data().users[0].name`
  rather than hard-coded); `user_named("nobody")` → `StoreError::NotFound { entity: "app_user", .. }`.
- `writer.rs:232` `an_offline_backend_hands_out_no_writer_and_no_user`: the `this_user` expectation
  becomes `NotFound` (empty mirror); the `writer().is_none()` half stays until T16 flips it.

### F-2. T16 — `crates/htui-store/tests/writer_buffered.rs` (no Postgres; `CacheStore::open` over `tempfile::tempdir()`)

Each case: `let writer = BufferedWriter::new(cache.clone())`, a `ChatRunSpec::mint(ids::PROJECT_HTUI,
ids::BOX, ids::USER, Some(ids::AGENT_CLAUDE), None)`, rows from a local `event(step, seq, kind)`
helper (the `tests/cache.rs:118` shape).

| Case | Asserts |
|---|---|
| `start_chat_run_registers_and_append_events_buffers_in_seq_order` | `run_of(step) == Some((project, run))`; after `append_events(&rows[..3])` then `&rows[3..]`, the file `pending/<project>.<run>.jsonl.open` holds 5 lines that parse back to `rows` in order; returns 3 then 2 |
| `an_event_for_an_unregistered_step_is_not_found_and_writes_nothing` | `NotFound { entity: "run_step" }`; `pending/` has no file — including when the batch also holds a registered step's rows |
| `a_batch_spanning_two_runs_lands_in_two_files` | two `start_chat_run`s, one `append_events` of interleaved rows → two files, each in its own `seq` order, sum returned |
| `item_and_registry_writes_are_unreachable` | `mint_item`, `update_item`, `transition`, `upsert_agent`, `upsert_agent_box` → `StoreError::Unreachable(_)` |
| `set_step_usage_is_a_no_op` | `Ok(())`, no file created |
| `finish_chat_run_seals_the_buffer` `[H-1]` | after append + finish: `.jsonl` exists, `.open` gone; a finish with nothing appended is `Ok` and creates nothing; an unregistered step is `NotFound` |
| `a_clone_shares_the_registration` | `start_chat_run` on the original, `append_events` on the clone → written |
| `reads_delegate_to_the_mirror` | `writer.step_events(step)` → `None` on the fresh mirror |
| `backend.rs` / `writer.rs` unit | `Backend::Offline.writer()` is `Some`, `label() == "buffered"`; `is_writable()` still `false`; `writable()` still `None` |

`tests/cache.rs` file-only cases beside `pending_append_*`: `pending_append_writes_the_open_name`
(existing three cases re-pointed at `.jsonl.open`), `seal_pending_renames_once`,
`seal_orphaned_adopts_open_buffers_at_open` (write a `.open`, re-`CacheStore::open` the same root,
the file is `.jsonl`), `upload_pending_ignores_open_buffers` (Postgres-gated: an `.open` file beside a
sealed one — only the sealed one lands, the `.open` one is still there).

### F-3. T17 — `htui-core` unit tests, `crates/htui-store/tests/pg_criteria.rs`

- `usage.rs` in-module: `from_rows` over `[prompt, usage{100,50,_,_,10}, other, usage{_,_,5,_,20}]`
  → `{100, 50, 5, null, 30}`; a never-reported key stays `null`; `i64::MAX + 1` saturates;
  `to_value()` key order is the five keys in declaration order; `serde` round trip.
- `pg_criteria.rs`: `an_uploaded_offline_chat_carries_its_digest_and_usage` (criteria 3 and 7): a
  buffer of a `prompt` row with `digest = "<64 hex>"`, two `assistant_text`, three `usage` rows and a
  `done`, written sealed → `upload_pending` → `run_step.prompt_digest == digest`,
  `run_step.usage == UsageTotals::from_rows(&rows).to_value()` (read back through `sqlx::query`,
  `Value` compared); a second `upload_pending` returns 0 files… **UNVERIFIED — implementer must
  check** the return value contract when the file was already deleted (it is `uploaded` count; the
  second pass finds no file and answers `0`); the rows and the two columns are unchanged.
- `tests/recorder.rs`, `fake_conformance.rs`, `acp_conformance.rs`: untouched and green — the proof
  that the `UsageTotals` move changed nothing.

### F-4. T18 — `crates/htui-agent/tests/replay.rs`

- `every_recorded_kind_decodes_to_the_event_that_produced_it`: a `Recorder` over `MemStore::demo()`
  and `MinimalScrubber::new([])` records a script covering **all eleven** `DriverEvent` variants
  plus `record_prompt`, `record_follow_up`, `record_permission_answer`; `step_events(step)` →
  for each row `envelope_from_row` is `Ok`; for the eight driver kinds
  `EventKind::from(&decoded.event) == row.kind`; for the three `htui` kinds and the banner the result
  is `Other { update: row.kind.as_str() | "session_started", body == row.payload }`; `at == row.at`.
- `rows_re_recorded_from_their_envelopes_are_the_same_rows` (the regression net): rows → `envelopes`
  → a second `Recorder` on a fresh step, routing `Other { "prompt" | "follow_up" |
  "permission_answer" }` to the three `record_*` calls and everything else to `record` → its rows
  equal the first set field for field except `run_step_id`. The `message_id` stamp is what makes
  the `assistant_text` rows come back one-for-one.
- `a_recorded_acp_turn_decodes_to_what_it_rendered`: `tests/fixtures/claude_acp_turn.jsonl` →
  `Mapper::map` (the `acp_map.rs:27-35` helper) → `Recorder` → rows → `envelopes` →
  `insta::assert_debug_snapshot!("replay__acp_turn", events)`. Adapter drift shows as a snapshot
  diff, the same net `acp_map.rs` casts one layer down.
- `the_fixture_plan_step_decodes`: `MemStore::demo().step_events(ids::STEP_PLAN)` (8 rows,
  `fixtures.rs:1222-1300`) → 8 envelopes, the `tool_call` / `tool_result` ids filled from the
  column, `insta::assert_debug_snapshot!("replay__fixture_plan_step", ..)`.
- `an_undecodable_payload_is_strict_err_and_lossy_other`: a `plan` row with `status: "weird"` →
  `Err(ReplayError { kind: Plan, seq, .. })`; `envelope_or_other` → `Other { update: "plan", body }`.
  A `done` row with `stop_reason: "later"` likewise. A row of kind `other` without `update` →
  `Other { update: "other" }`.
- `envelopes_sorts_by_seq`: a shuffled slice comes back in `seq` order.

### F-5. T19, T20 — `crates/htui/tests/replay.rs` (harness, `testkit` feature)

Harness: `Harness::demo().with_tab(Box::new(BacklogTab::new())).with_tab(Box::new(ChatTab::new()))
.with_replay_tab(ChatTab::ID)`. Navigation to `FEAT-1` and the Runs pane follows
`tests/backlog.rs`'s `detail_runs` case (**UNVERIFIED** the exact key sequence; the snapshot at
`backlog__detail_runs.snap` shows the target state).

| Test | Task | Asserts / snapshot |
|---|---|---|
| `store_worker` in-module: `step_events_is_served_as_a_read` | T19 | `serve(&Backend::memory(demo), &StepEvents(ids::STEP_PLAN))` → `StepEvents { events: Some(8) }`; a fresh `StepId::new()` → `events: None` (`MemStore::step_log` answers `None` when no row matches, `mem.rs:540-542`); `name() == "step_events"` |
| `runs_pane_selects_a_step` | T19 | after `l`… to Runs, `J` → `backlog__runs_step_selected`; re-accept `backlog__detail_runs` |
| `enter_on_a_step_focuses_the_chat_tab` | T19 | `Enter`, `drive()`, `harness.app().tabs.active_id() == Some(ChatTab::ID)`; `Enter` on the Body pane → status line carries the keymap refusal text |
| `enter_on_a_project_header_still_folds` | T19 | the existing fold behaviour is untouched |
| `replay_of_the_fixture_step_renders_its_rows` | T20 | `chat__replay_fixture_step`: header `replay · step …`, the prompt line `you  Plan the TUI scaffold…`, the tool call `Read docs/ANA-9.md` completed, the plan, the usage line, `done end_turn` |
| `replay_of_a_step_not_on_this_box` | T20 | `harness.app().update(Action::Replay { step_id: StepId::new() })`, `drive()` → `chat__replay_missing` with `this step is not on this box` |
| `replay_rows_equal_the_live_rows` | T20 | run the `chat.rs` streamed-turn script to its end (`harness_with`, `chat_steps()[0]`), `render()` → `live`; `update(Action::Replay { step_id })`, `drive()`, `render()` → `replay`; the body lines (everything between the header line and the hint line) are **equal**; snapshot `chat__replay_streamed_turn` |
| `replay_is_read_only` | T20 | in replay: `i`, `Enter`, `1`, `a` → `drive()` issues no request (the request channel is empty — `Harness` exposes the count **UNVERIFIED**; otherwise assert the render is unchanged and `chat_steps().len()` did not grow); `Esc` → the live transcript is back, `chat__replay_leave` |
| `a_cancelled_permission_replays_parked_but_unanswerable` | T20 | the `chat_permission_inline` script cancelled mid-park (`Esc Esc`), then replay: the `Permission` row renders unresolved, no strip, `1` does nothing — `chat__replay_cancelled_permission` (D41) |
| `transcript.rs` in-module: `from_rows_over_the_fixture` | T20 | 8 rows → `len() == 6` (`tool_result` folded, `done` and `usage` are rows; **UNVERIFIED** count — derive from `apply`'s rules) |

### F-6. T21 — `crates/htui/tests/chat_offline.rs` and `htui_store::testkit`

Why the promotion: §11 criterion 12 is one sentence with two halves — a **driver** writes the
buffer, a **server** lands it. `htui-store` cannot see a driver (constraint), `htui-agent` has no
store, and `htui` has no throwaway-database harness. Moving `crates/htui-store/tests/common/mod.rs`
to `crates/htui-store/src/testkit.rs` under `test-support = ["demo"]` (the `htui-core` precedent:
`fixtures` under the same feature name) makes it reachable from `htui`'s tests; the five
`htui-store` test files change one line each. Added there:

```rust
/// Rows a test puts into a mirror without a server: the own `box`, one `app_user`, the agents.
/// Writes through `cache.pool()` with the mirror's own encodings (`ts_bind`, JSON text), so a test
/// never hand-rolls a SQLite row. `test-support` only.
pub async fn seed_mirror(cache: &CacheStore, user: &AppUser, box_row: &BoxRow, agents: &[Agent]) -> Result<()>;
```

(**UNVERIFIED — implementer must check** the exact `BoxRow` field set against `box_.rs` and the
`refresh_box` bind list, `refresh.rs:513-533`.)

`chat_offline.rs`:

1. `tempdir` → `CacheStore::open(root, "offline-chat", PgStore::schema_version())` →
   `seed_mirror(&cache, fixture user named os_user_name(), fixture box, &[scripted_row(..)])`
   (the `chat.rs:40` row: `cli`, `settings.cli.stream = "fake"`).
2. `Harness::over_backend(Backend::Offline { cache: cache.clone(), since: Some(Utc::now()) })
   .with_tab(ChatTab).with_agent_runtime(..)` — the `chat.rs` streamed-turn script.
3. `i`, prompt, `Enter`, `drive()` → `chat__chat_buffered`: the header ends
   `· buffered · uploads when the store returns`; the transcript is the live one.
4. `Esc Esc`, `drive()` → the file `pending/<PROJECT_HTUI>.<run>.jsonl` exists (sealed `[H-1]`; before
   the cancel it was `.jsonl.open`), its lines parse to `SessionEvent`s with gapless `seq` from 0,
   kinds `prompt, other(session_started), assistant_text…, done`, all `run_step_id ==
   harness.chat_steps()[0]`; no `.open` file remains.
5. Postgres tail, `if let Some(db) = testkit::demo_db().await`: `upload_pending(&db.pool,
   cache.dir(), db.store.this_box(), db.store.this_user())` → `1`; `db.store.step_events(step)` ==
   the file's rows; `run_step.prompt_digest` == the prompt payload's `digest`; `run_step.usage` ==
   `UsageTotals::from_rows(&rows).to_value()`; the file is gone; a second upload is `0` and changes
   nothing; `db.drop_db()`. Without the variable the tail prints `testkit::SKIP` — the plan's
   standing rule.
6. `agent_worker.rs` in-module (beside M3's T13 step-4 tests, **UNVERIFIED** names): the offline
   `serve(ChatStart)` no longer answers `Failed`; `run_chat` over `Writer::Buffered` calls
   `set_step_usage` (no-op) without error and seals at the end.

---

## G. Build order and checkpoints (serial edges: T15 → T16 → T17; T18 ∥ T17; T19 after T18; T20 after T19; T21 after T16 and T20)

Every checkpoint: `cargo fmt --all -- --check` first. Postgres steps carry
`USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres`; `.sqlx`
steps run from inside `crates/htui-store` with
`DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features`.

### T15 — mirror the registry and the offline user

| # | Step | Proof |
|---|---|---|
| 1 | Tests first (F-1): the three `cache.rs` cases and the `writer.rs:232` amendment — red | `cargo test -p htui-store --all-features --test cache` (Postgres) |
| 2 | `0002_agent_mirror.sql`; `MIRRORED_TABLES` 16; `identity::os_user_name` + the `pg/mod.rs` call site | `cargo test -p htui-store --features demo` — the three rebuild cases now sweep 16 tables |
| 3 | `refresh.rs` `replace_agent` (+ `.sqlx`) | `cargo sqlx prepare --check`; the pass test green |
| 4 | `read.rs` `agents` / `this_user` / `user_named`; `bool_col`'s `expect` removed | offline cases green; `cargo clippy -p htui-store --all-targets --all-features -- -D warnings` |
| 5 | `backend.rs` arms and the `agents()` doc | `cargo test -p htui-store --all-features` (Postgres); `cargo doc -p htui-store --no-deps` |

Commit: `feat(store): mirror the agent registry; the offline user from the mirror`.

### T16 — `Writer::Buffered`

| # | Step | Proof |
|---|---|---|
| 1 | `tests/writer_buffered.rs` (F-2, all cases) and the `cache.rs` file-only seal cases — red | `cargo test -p htui-store --features demo --test writer_buffered` |
| 2 | `pending.rs`: `OPEN_SUFFIX`, `append_pending` on the open name, `seal_pending`, `seal_orphaned`; `CacheStore::open` calls `seal_orphaned` `[H-1]` | the `pending_append_*` and seal cases green |
| 3 | `writer.rs`: `BufferedWriter`, the two impls, `label`, module doc | `writer_buffered` green |
| 4 | `backend.rs`: `writer()` Offline arm, `is_writable` / module docs; `traits.rs` doc lines | `cargo test -p htui-store --features demo`; clippy; `pg_conformance.rs` `EXPECTED_CASES` unchanged |

Commit: `feat(store): Writer::Buffered — the offline session sink over pending/`.

### T17 — `upload_pending` fills the step columns

| # | Step | Proof |
|---|---|---|
| 1 | `usage.rs` unit tests (F-3) — red; `pg_criteria.rs` case — red | `cargo test -p htui-core`; Postgres line |
| 2 | `htui_core::model::usage`, `mod.rs` export; `record.rs` switched to it (`add_payload`, `record_unreadable` arm) | `cargo test -p htui-agent --features test-support` — recorder and both conformance suites unchanged |
| 3 | `upload_one` widened insert (+ `.sqlx`) | `cargo sqlx prepare --check`; `pg_criteria` green; `cache.rs` upload cases still green (criterion 12's "the existing MOD-6 test still passes") |

Commit: `feat(core,store): UsageTotals in htui-core; upload_pending writes prompt_digest and usage`.

### T18 — the replay decoder (parallel with T17: disjoint files)

| # | Step | Proof |
|---|---|---|
| 1 | `tests/replay.rs` (F-4), snapshots absent — red | `cargo test -p htui-agent --features test-support --test replay` |
| 2 | `replay.rs` per B.9 and E; `lib.rs` exports | green; `cargo insta review` the two snapshots; clippy; `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings` (the plan's Windows gate) |

Commit: `feat(agent): replay — session_event rows back into DriverEnvelopes`.

### T19 — `StepEvents` and the way into replay (after T18)

| # | Step | Proof |
|---|---|---|
| 1 | `store_worker` unit test; `tests/replay.rs` T19 cases (F-5) — red | `cargo test -p htui --features testkit` |
| 2 | `StoreRequest::StepEvents` / `StoreReply::StepEvents`, `name`, `try_serve` | unit test green |
| 3 | `Action::Replay`; `App.replay_tab` + `register_all`; `App::replay`; `Harness::with_replay_tab` | focus test green |
| 4 | `runs.rs` step rows + cursor + `Enter`; `backlog/mod.rs` `Enter` fall-through; `keymap.rs` binding | `backlog__runs_step_selected` accepted; `backlog__detail_runs` re-accepted (step lines only) |

Commit: `feat(tui): StepEvents, Action::Replay and step selection in the Runs pane`.

### T20 — Chat tab replay mode (after T19)

| # | Step | Proof |
|---|---|---|
| 1 | The five T20 cases of F-5 — red | `cargo test -p htui --features testkit --test replay` |
| 2 | `Transcript::from_rows` / `len`; `ReplayState`; `on_key` branch; `on_reply` arm; render | green; `cargo insta review` the four `chat__replay_*` snapshots; `rg 'ctx\.(request|emit)'` inside the replay branch of `chat/mod.rs` finds nothing |

Commit: `feat(tui): read-only replay in the Chat tab`.

### T21 — the offline chat (after T16 and T20)

| # | Step | Proof |
|---|---|---|
| 1 | `htui_store::testkit` promotion (+ `seed_mirror`), five `use` lines, `htui` dev-dep | `cargo test -p htui-store --all-features` unchanged and green (Postgres) |
| 2 | `tests/chat_offline.rs` (F-6) — red | `cargo test -p htui --features testkit --test chat_offline` |
| 3 | `agent_worker.rs`: refusal gone, `writer_label`; `ChatAccepted` field; `chat/mod.rs` `buffered` header; `Harness::over_backend` | `chat__chat_buffered` accepted; the eight `chat__*` snapshots unchanged (`"memory"` adds no suffix) |
| 4 | Full gate: `cargo clippy --workspace --all-targets --all-features -- -D warnings`; `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features`; `cargo doc --workspace --no-deps`; docs row 37 | the plan's Validation block, Postgres line included |
| 5 | Live run: start `htui` with Postgres stopped → `4` → prompt against `claude` → header says `buffered` → `Esc Esc` → `ls <cache_dir>/pending/` shows one `.jsonl` → start Postgres → within `RECONNECT` the file is gone and `psql` shows the `run_step` with `usage` and `prompt_digest`; `1` → FEAT-1 → Runs → `J` → `Enter` → the replay | the milestone write-up quotes both |

Commit: `feat(tui,store): the offline chat over Writer::Buffered, uploaded on reconnect`.

---

## H. Risks

| # | Risk | Mitigation in this design | Anticipated by the plan? |
|---|---|---|---|
| H-1 | A chat started offline that is **still running** when the store comes back: the refresher's next pass uploads its partial buffer (`upload_pending` runs every pass, `refresh.rs:297-298`), writes `run.status = 'done'` and `run_step.usage` from a partial sum, and deletes the file; the chat keeps appending to a recreated file whose later upload hits `ON CONFLICT DO NOTHING` on the step — `usage` frozen at the partial value (criterion 7 broken), `finished_at` wrong | The `.open` suffix (B.5): `append_pending` writes it, `finish_chat_run` seals (**deviation from D35**'s "no-op"), `upload_pending`'s existing extension filter never sees an open buffer, `seal_orphaned` at `CacheStore::open` covers a crash. Three items, one file owner, marked `[H-1]` throughout | **No** |
| H-2 | D39's `j`/`k` are the Backlog list's keys and `Enter` never reaches the pane | `J`/`K` and the `Enter` fall-through (preamble, B.12) | **No** — plan ≠ tree |
| H-3 | An uploaded offline chat has `run_step.agent_id` and `model` `NULL`: the line format carries neither (`pending.rs:14-17`) | Stated in `upload_one`'s doc; the banner row holds `agent_name` / `models`, so a later milestone can backfill from it without a format change | No |
| H-4 | A replay reply decodes on the UI task | Serde over rows already in memory, no I/O; the fallback (decode in `try_serve`) keeps the reply type (C-2) | No |
| H-5 | Two `assistant_text` rows glued into one transcript row on replay | The `seq:N` `message_id` stamp (E) | No |
| H-6 | The demo fixture's `tool_call` / `tool_result` payloads carry no `tool_call_id` key (§4.3 shape), unlike recorder rows | The column-fill rule (E); the fixture snapshot test pins it | No |
| H-7 | `USERNAME` at test time is not the fixture user's name, so `this_user()` on a demo mirror is `NotFound` | `user_named(name)` is the tested unit; `this_user()` is a one-line composition over `os_user_name()` | No |
| H-8 | Offline `Settings > Agents` now lists rows with `on_box: None` ("not probed") where it used to show a refusal | Stated in `Backend::agents()`'s doc. `settings/agents.rs:74` keeps its `Failed { request: "agents" }` arm (a genuine server error still needs it); only its comment at `:30` and `:71-73` ("since `agent` is not mirrored") is rewritten — docs-only, row 37. The `settings__agents_*` snapshots run over `Memory`, so none moves | Partly (D31 names `on_box: None`) |
| H-9 | `StoreReply::ChatAccepted` gains a field: every constructor and every pattern (`chat/mod.rs:322`, `agent_worker.rs:573`, tests) | Exhaustive match makes the compiler list them; the field is `&'static str` so no snapshot moves except the buffered one | No — file-set addition (T21, `store_worker.rs`) |
| H-10 | `replace_agent` adds a `query!` in T15, so `.sqlx` must regenerate one task earlier than the plan says | A.6, G T15 step 3 | **No** — plan gap |
| H-11 | The `htui-store` `testkit` promotion touches five test files' preambles | Mechanical, one line each; the alternative (a captured fixture as the joint between crates) would not fail when the recorder and the uploader drift | No — file-set addition (T21) |
| H-12 | The replay hint / header text is new UI copy nobody has approved | Two lines, quoted in B.13, reviewed with the snapshots | No |

---

## I. What this milestone does NOT touch

- **Milestone 5 (probe)**: `crates/htui-agent/src/probe.rs` does not exist and is not created;
  `crates/htui-store/migrations/0002_agent_probe.sql` is not written — the Postgres migration
  directory stays at one file; `agent_box` is neither mirrored (D31) nor written; `AgentSummary.on_box`
  offline is `None` by construction, not by a probe that found nothing.
- **MOD-4**: its cache migration is now `cache_migrations/0003_orchestration.sql` (ANA-2 §9's
  `0002_orchestration.sql` number is taken by A.1); nothing here reads or writes `run.graph_snapshot`,
  gate outcomes or the command queue. The Runs pane's step rows render `RunStepSummary` fields that
  already exist; approve / reject stay `DetailTab::on_key` bodies MOD-4 adds.
- **MOD-13**: offline item editing — `mint_item`, `update_item`, `transition` on the buffered writer
  stay `Unreachable` (D35); `Backend::writable()` stays `None` off the server; no `pending/` format
  for anything but `session_event` rows.
- **Milestone 9 / ANA-5**: the `prompt` payload's `sections` and the digest rule are consumed as
  the recorder wrote them, never assembled here.
- **Milestone 7**: `_always` grants and quota latching — a replayed `permission_answer` renders
  what the row says (`by`, `option_id`) and nothing is remembered.
- **The store seam**: no method is added to `ReadStore` or `WriteStore` (`pg_conformance.rs`
  `EXPECTED_CASES` unchanged); no `Tab` trait method (D39); `event_loop.rs` unchanged (ANA-4 §8's
  promise).
- **The scrubber**: it runs in `Recorder` only; `append_pending`, `upload_pending`,
  `UsageTotals::from_rows` and `replay::*` read payloads as opaque JSON or as the structs the
  recorder wrote, and none of them masks, unmasks or inspects text.
