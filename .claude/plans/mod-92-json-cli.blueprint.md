# Blueprint: MOD-92, read-only `--json` CLI (`htui status`, `htui waiting`, `--search-items --json`)

**Plan:** `.claude/plans/mod-92-json-cli.plan.md` (confirmed, fact-checked, D1-D8). Produced by `code-architect`.
Line numbers are for `8e620755`; they drift after the first commit, so anchor on names.

- T1 (htui-store) and T2 (htui-worker + four htui test literals) are independent and may run in parallel.
- T3 (htui) runs after both are merged. T4 (docs) runs after T3, because it copies T3's snapshots.
- **No new `sqlx::query!` anywhere.** T1 moves existing macro calls without changing their SQL, so the
  offline hash (the literal query) is unchanged. T3 only calls existing `PgStore` methods. The tests
  use runtime `sqlx::query(..)`. `cargo sqlx prepare --workspace --check` stays green without a
  re-prepare; run it in the gate anyway.

---

## Plan amendments and clarifications

Items marked **PLAN AMENDMENT NEEDED** change or contradict the plan's text and need the maintainer's OK.
The rest are blueprint decisions in areas where the plan says nothing.

- **A-1 (PLAN AMENDMENT NEEDED, D3: "every outcome prints exactly one JSON document").** Three
  outcomes cannot print a document. Document them in `docs/htui-cli.md`, and MOD-93 must treat an
  empty stdout as a failure:
  - a clap usage error: exit 2, usage on stderr, raised before `run`;
  - an unopenable `--log` file: exit 1, raised by `init_tracing`, which runs before dispatch;
  - a panic: exit 101.
- **A-2 (PLAN AMENDMENT NEEDED, D3 table extended for search).** `store` describes Postgres only. A
  search whose Postgres answered always has `"store":"online"` and adds an `index` state:

  | `index` | When | `hits` | `error.code` | exit |
  |---|---|---|---|---|
  | `online` | collection exists and answered | array | absent | 0 |
  | `empty` | collection does not exist (plan G8) | `[]` | absent | 0 |
  | `offline` | Qdrant open or search failed, not a mismatch (today's "cannot reach Qdrant") | absent | `qdrant_unreachable` | 3 |
  | `refused` | `is_embedder_mismatch` | absent | `embedder_mismatch` | 2 |
  | `error` | embedding model could not be fetched or loaded | absent | `model_unavailable` | 1 |

  A missing, unreadable or unusable Qdrant URL is checked before Postgres, as `concepts::open` does
  today. It gives a failure document, `"store":"refused"`, with code `no_qdrant_url`,
  `keyring_unreadable` or `qdrant_url_unusable`, exit 2.
- **A-3 (PLAN AMENDMENT NEEDED, D6 covers output only).** `--search-items` without `--json` goes
  through the same `StateExit`, so its exit codes change from "1 for everything" to 3 when
  Postgres/Qdrant is unreachable and 2 for a refusal. The stderr sentences stay the same:
  `headless_refusal` and `qdrant_refusal` are reused. Recommendation: accept, because exit codes
  that differ by mode would be a trap. The alternative is mapping every human-mode failure back to 1.
- **A-4 (PLAN AMENDMENT NEEDED, "No writes").** `concepts::open` reads `box.toml` through
  `identity::config_root()`, which runs `create_dir_all`. The verbs must use
  `identity::config_root_path()` (no create) and the new `identity::load`. With `config_root_path`
  `Err` (no config directory on this platform), the box is `null`.
- **A-5 (PLAN AMENDMENT NEEDED, T1 file list).** The plan's T1 says the `open_existing` cases go "beside
  the existing Qdrant cases", but `crates/htui-store/tests/qdrant_live.rs` is missing from its
  file list. Add it. It is still disjoint from T2.
- **A-6 (PLAN AMENDMENT NEEDED, validation command).** `cargo test -p htui --all-features waiting
  top_bar` does not parse, because cargo takes a single TESTNAME. Use `cargo test -p htui
  --all-features --lib -- waiting top_bar`; libtest takes several filters.
- **A-7 (fact).** "16 `WaitingRow` literals" is really 10 struct literals: views.rs 4 (2
  production at `:624`, `:765`; 2 test at `:1445`, `:1923`), store_worker.rs 1 (`:3811`), top_bar.rs 1
  (`:86`), waiting_list.rs 4 (`:433`, `:445`, `:453`, `:616`). The other 6 grep hits are the struct
  definition and helper signatures. The four files are correct.
- **A-8 (clarification, D3 "refused").** Any `HeadlessError::Store(e)` at connect where `e` is not
  `Unreachable` becomes refused/2 with code `store_refused`. This covers a newer or drifted schema,
  a partial migration, a malformed target, a failed password (`28P01` → `Backend`) and a missing
  database (`3D000`). It matches `concepts::store_context`'s "Postgres refused this htui". The
  `error`/1 outcome is only for a read that fails *after* the connect with something other than
  `Unreachable`.
- **A-9 (clarification, `--cwd`).** A deepest match that names two or more projects (one checkout
  path registered for repos of different projects) is refused/2 with code `cwd_ambiguous`. htui
  does not pick one. `--cwd` without a `box.toml` is refused/2 with code `no_box`, because the box id
  decides which `repo_box_path` rows are "this box's".
- **A-10 (clarification, D2 pool size).** `PoolSize::MIN` is 2 and the field is private. Use
  `PoolSize(PoolSize::MIN)` inside `pg/mod.rs` and add no public constant.
- **A-11 (clarification, D4 timestamps).** Serialise as `to_rfc3339_opts(SecondsFormat::Secs,
  true)`, for example `2026-10-08T20:01:02Z`: whole seconds, always `Z`. The plan's example already
  has this shape, and it keeps snapshot filters to one regex.

---

## Design decisions

- **D-A: the seam is "answer, then emit".** `state_cmd::answer` is async and returns data (`Answer`). It
  never writes and never fails. `state_cmd::emit` is sync and writes to `&mut dyn Write`. This has
  two benefits:
  - `StdoutLock` is never held across an `.await`.
  - The Pg tests assert on `Answer`/`emit` into a `Vec<u8>`, without the keyring and without stdout.

  `run` is the only function that reads the keyring, `box.toml` and the clock.
- **D-B: reuse rather than re-derive.**
  - `htui_worker::waiting` does the classification.
  - `store_worker::permissions_or_unknown` decides permission-read failures. T2 makes it `pub(crate)`.
  - `concepts::headless_refusal` gives the message text, and `concepts::qdrant_refusal` gives the
    Qdrant sentence.
  - `concepts::scopes` scopes the search. T3 makes it `pub(crate)`.
  - `concepts::format_hit` gives the human search lines.
  - `print_keys`' BrokenPipe rule is extracted into `lib.rs::write_out`.
- **D-C: `since` is `updated_at`, per plan D5. The engine's park was checked:**
  - **The gate park does not set `finished_at`.** `PgStore::park_step` (`pg/write.rs:6091`) runs
    `UPDATE run_step SET status = 'awaiting_approval'`, then `UPDATE run SET status =
    'awaiting_approval'`, then `UPDATE item SET status = 'awaiting_approval'`. It does not touch
    `finished_at`.
  - **`finished_at` comes from `finish_step`** (`:5328`, `finished_at = $7`), which runs earlier
    with the walker's clock, when the agent ended. It is not the park.
  - **A promoted step** (`:5986`) sets `promoted_at` but has no `finished_at`.
  - **Every `UPDATE` stamps `updated_at`.** The BEFORE UPDATE trigger `set_updated_at()`
    (`0001_init.sql:21`, loop at `:575` over `item`, `run`, `run_step`, ...) sets `updated_at :=
    clock_timestamp()`. So the park's own `UPDATE` stamps the park instant on the step, the run
    and the item, on the store's clock.
  - **The permission row uses the same clock.** `StepPermission.created_at` is documented as "the
    store's clock (I-4)", so all six reasons read one clock.
  - **After the park:**
    - `release_lease` (`:5100`) bumps `run.updated_at` again at the same instant.
    - The sweep `adopt_runs` (`:4972`) takes only `status = 'running'`, so it never re-stamps a
      parked run.
    - `take_lease` (`:5048`) does accept `awaiting_approval`, but only when a person acts on the
      run, and that ends the wait.
  - So `updated_at` is the park instant up to the documented caveat (a later edit of the row
    re-stamps it). `finished_at` is wrong for promoted steps, absent on some rows, and on another
    clock.
  - **Per reason:**

    | Reason | `since` |
    |---|---|
    | Gate | `step.updated_at` |
    | JudgeFailed | `judge.updated_at` |
    | Selection | `run.updated_at` |
    | Unblock FollowRun/Resume | `run.updated_at` |
    | Unblock Reopen | `item.updated_at` (the `blocked` move re-stamps it through the trigger) |
    | Interrupted | `run.updated_at` |
    | Permission | `permission.created_at` |
- **D-D: the age uses this process's clock.** `age_seconds = max(0, now - since)`, with `now =
  Utc::now()` taken once in `run`. Clock skew between this box and the database server shifts the
  age, and the floor at 0 absorbs negative skew. Document this. Reading the database's `now()`
  would need a new `query!` and a `.sqlx` entry, so it is out of scope.
- **D-E: stdout output is one compact line**: `serde_json::to_writer` plus `\n`, for `jq` and line
  readers. Snapshots pin `serde_json::to_string_pretty` of the same value, and one test pins that
  `emit` writes exactly one line that parses back to that value.

---

## T1: read-only store session (htui-store)

**Files:** `crates/htui-store/src/pg/mod.rs`, `crates/htui-store/src/identity.rs`,
`crates/htui-store/src/vector.rs`, `crates/htui-store/tests/read_only_pg.rs` (new),
`crates/htui-store/tests/qdrant_live.rs` (A-5).

### Tests first

`crates/htui-store/tests/read_only_pg.rs`:
- Header: `#![cfg(feature = "test-support")]` and `use htui_store::testkit as common;`, as
  `migrations.rs` does.
- Each case does `let Some(db) = common::fresh_db().await else { return; };` (the helper prints
  `SKIP`) and ends with `db.drop_db().await`.
- Use a `const WAIT: Duration = Duration::from_secs(5)`, like `migrations.rs`'s `HEADLESS_WAIT`.

| Test | Asserts |
|---|---|
| `a_read_only_session_answers_reads` | `connect_read_only(&db.url, Some(db.identity.box_id), WAIT)` gives `Ok`. `workspaces()` gives `Ok`. `this_box()` equals the given id, `this_user()` is nil, `registration()` is `None`. |
| `a_raw_insert_is_refused_by_the_session` | `sqlx::query("INSERT INTO workspace (slug, name) VALUES ('x','x')")` (check `0001_init.sql` for the NOT NULL columns and pick a minimal valid row) on `store.pool()` gives a `sqlx::Error::Database` with `code() == Some("25006")` (`read_only_sql_transaction`). |
| `a_store_write_is_refused_and_writes_nothing` | A `WriteStore` write through the read-only store (for example `create_workspace` with a fresh `NewWorkspace`) is `Err(StoreError::Backend(m))` with `m.starts_with("25006")`. The `workspace` row count through `db.pool` is unchanged. |
| `a_read_only_connect_registers_and_seeds_nothing` | Counts `box`, `app_user` and `capability_tag` through `db.pool`. Then runs `connect_read_only` with a **new** `BoxId::new()` (one `connect_headless` would insert) and with `None`. The three counts are unchanged, and no `box` row has the new id. |
| `a_pending_schema_is_refused_read_only` | Over `common::bare_db()`: `HeadlessError::MigrationsPending(n)` with `n` equal to the embedded up-migration count (17 today; assert against the migrations.rs constant if one exists, else the literal with the same comment). `_sqlx_migrations` has 0 rows. The public table count is unchanged. |
| `a_build_below_the_target_is_refused_read_only` | Raises `app_setting.htui_target_version` to `"999.0.0"` through `db.pool`, as `migrations.rs` `a_headless_connect_below_the_target_refuses` does. The result is `HeadlessError::BelowTarget { .. }`. |
| `an_unreachable_server_is_unreachable_read_only` | `postgres://nobody:nothing@127.0.0.1:1/none` with a 2 s timeout gives `HeadlessError::Store(StoreError::Unreachable(_))`. Needs no database: it never skips. |
| `every_cli_read_answers_on_a_read_only_session` | Over `common::demo_db()` with `Some(ids::BOX)`, every read T3 uses returns `Ok`: `workspaces`, `box_info`, `active_runs(&scope)`, `ReadStore::waiting_candidates(&scope)`, `WriteStore::open_permissions(&scope)`, `repo_paths(ids::BOX)` and `WriteStore::repos(project)`, with `scope = Scope::from_workspace(first)`. **This catches a hidden `FOR UPDATE` or write in a read path at T1, not at T3** (the plan's top risk). |

`identity.rs` `mod tests` (after `load_or_mint_still_names_the_path`, `:576`):

| Test | Asserts |
|---|---|
| `load_on_a_missing_file_is_none_and_writes_nothing` | On a tempdir: `Ok(None)`. `read_dir` is empty afterwards. A missing **root** directory is also `Ok(None)` and is not created. |
| `load_returns_the_id_and_this_hostname_without_rewriting` | Calls `store(root, &Identity { box_id, hostname: "old-host" })` and keeps the bytes. `load` returns `Some(Identity { box_id, hostname: hostname() })`. The file bytes are unchanged. |
| `load_names_the_path_of_a_malformed_file` | Garbage text gives `Err(StoreError::Backend(m))`, with `m` containing the path and `"is not valid box.toml"` (the sentence `load_or_mint` uses). |

`tests/qdrant_live.rs`, after `another_dense_width_is_refused_and_not_stamped` (`:505`). Each case
uses a `fresh_name()` collection and the file's existing `raw`, `create_unstamped` and
`stored_identity` helpers, and drops what it created:

| Test | Asserts |
|---|---|
| `open_existing_on_a_missing_collection_is_none_and_creates_nothing` | `open_existing_to(..)` gives `Ok(None)`, and `raw(url).collection_exists(name)` is `false`. |
| `open_existing_searches_what_connect_built` | `connect_to` plus one `Indexer::sync` over `MemStore` (as `sync_then_search_finds_an_item_by_its_exact_key` does). `open_existing_to` gives `Some`, and its `search` finds the key. |
| `open_existing_refuses_another_identity` | A collection stamped by another `EmbedderIdentity` gives `Err(e)` with `is_embedder_mismatch(&e)`. |
| `open_existing_refuses_another_width` | `create_bare(.., dim + 1)` gives `is_embedder_mismatch`. |
| `open_existing_leaves_an_unstamped_collection_unstamped` | `create_unstamped` gives `Ok(Some(_))`, and afterwards `stored_identity(&client, name)` is still `None`. No stamp is written. |

### `crates/htui-store/src/pg/mod.rs`

1. **`open_pool` gains a session kind.** Add a private enum before `open_pool` (`:792`):

   ```rust
   /// How a pool's sessions start (MOD-92 D2).
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   enum Session {
       /// The TUI's and the headless processes' sessions.
       ReadWrite,
       /// `default_transaction_read_only = on`, sent in the startup packet: Postgres refuses every
       /// write, temporary tables included (`25006`).
       ReadOnly,
   }

   async fn open_pool(dsn: &str, connect_timeout: Duration, pool: PoolSize, session: Session) -> Result<PgPool>
   ```

   - In the body, after `from_str`: `let options = match session { Session::ReadWrite => options,
     Session::ReadOnly => options.options([("default_transaction_read_only", "on")]) };`.
   - `options()` appends its `-c` after any `options=` the DSN carries, so ours is the last one and
     wins. Say so in the doc.
   - Update the two existing callers: `connect_with` (`:249`) and `connect_headless` (`:291`) pass
     `Session::ReadWrite`.
2. **Extract the headless refusal checks** from `connect_headless`, unchanged, into a private
   function placed after `migrations_table_exists`:

   ```rust
   /// The refusals of a process with no one to ask (MOD-40 blueprint B19), every one a read:
   /// a pending schema (without creating the migrations table), a dirty, newer or drifted one, a
   /// malformed target, a build below the target. Shared by `connect_headless` and
   /// `connect_read_only` (MOD-92 D2), so the two cannot disagree.
   async fn headless_checks(pool: &PgPool) -> core::result::Result<(), HeadlessError>
   ```

   - The body is lines `:292-315` moved verbatim: acquire, `migrations_table_exists`,
     `applied_state` / `embedded_migrations`, drop the connection, `MigrationsPending`,
     `stored_target` + `parse_target` + `BelowTarget`.
   - `connect_headless` becomes `open_pool(.., Session::ReadWrite)` → `headless_checks(&pool).await?` → `Self { .. }` → `bootstrap`.
   - No SQL text changes, so no `.sqlx` change.
3. **New constructor**, placed right after `connect_headless` (before `lazy`):

   ```rust
   /// Connects for a read-only verb (MOD-92 D2): `htui status`, `htui waiting`, `--search-items`.
   ///
   /// [`connect_headless`](Self::connect_headless)'s pool and refusals, then **no** bootstrap:
   /// nothing is seeded and no box is registered. Every session of the pool starts with
   /// `default_transaction_read_only = on`, so Postgres itself refuses a write, whatever code path
   /// attempts it. `this_box` is `box_id` as `box.toml` records it, or nil; it is never checked
   /// against the `box` table (a copied `box.toml` is not detected, MOD-7 D3, because that check
   /// registers). `this_user` is nil. `identity()` carries `box_id` and an empty hostname: a
   /// read-only store has no identity to persist. The pool holds [`PoolSize::MIN`] connections.
   ///
   /// # Errors
   ///
   /// As [`connect_headless`](Self::connect_headless): [`HeadlessError::Store`] (unreachable,
   /// newer, drifted, partial, malformed target, or any connect refusal such as a failed password),
   /// [`HeadlessError::MigrationsPending`], [`HeadlessError::BelowTarget`].
   pub async fn connect_read_only(
       dsn: &str,
       box_id: Option<BoxId>,
       connect_timeout: Duration,
   ) -> core::result::Result<Self, HeadlessError>
   ```

   The body is `open_pool(dsn, connect_timeout, PoolSize(PoolSize::MIN), Session::ReadOnly)`, then
   `headless_checks(&pool)`, then `Self { pool, identity: Identity { box_id:
   box_id.unwrap_or_default(), hostname: String::new() }, this_box: box_id.unwrap_or_default(),
   this_user: UserId::default(), registration: None, below_target: None }`.
4. Update the module doc's first line to mention the read-only connect.

### `crates/htui-store/src/identity.rs`

Add this after `load_or_mint` (`:83`):

```rust
/// Reads `<root>/box.toml` and never writes (MOD-92 G2): `Ok(None)` when there is no file (and
/// when `root` itself does not exist). The identity carries this machine's current hostname, as
/// [`load_or_mint`] returns it, but a changed hostname is **not** written back.
///
/// # Errors
///
/// [`StoreError::Backend`] when the file exists but cannot be read or parsed; the sentence names
/// the path, as [`load_or_mint`]'s does.
pub fn load(root: &Path) -> Result<Option<Identity>>
```

**Refactor shape.** Add a private `fn read_box_toml(path: &Path) -> Result<Option<BoxToml>>` that holds
today's `read_to_string` match, with `NotFound` mapped to `Ok(None)`. It keeps the two sentences byte
for byte: `"{} is not valid box.toml: {e}"` and `"cannot read {}: {e}"`.
- `load_or_mint` calls `read_box_toml`.
- On `Some(parsed)`: compare the hostname, call `store` if it changed, return.
- On `None`: mint and `store`.
- `load` calls `read_box_toml` and maps the result to `Identity { box_id, hostname: hostname() }`.
- Existing tests stay green unchanged.

### `crates/htui-store/src/vector.rs` (inside `impl<E: DenseEmbedder> QdrantStore<E>`, `:652`)

**Can it share `connect_to`'s mismatch check without `ensure_collection`? Yes.** The check is
`check_existing` (pure) fed by one `collection_info` read. Only the `Existing::Stamp` arm of
`check_collection` writes. Refactor:

1. Add a private `fn new(settings: &QdrantSettings, embedder: E, collection: &str) -> Result<Self,
   StoreError>`. It holds `connect_to`'s builder lines `:671-680`, including
   `.skip_compatibility_check()`, which is what keeps stdout clean (its comment says why).
   `connect_to` becomes `let store = Self::new(..)?; store.ensure_collection().await?; Ok(store)`.
2. Split `check_collection` (`:725`). Add a private `async fn existing(&self) -> Result<Existing,
   StoreError>`, which is the `collection_info` read plus `check_existing`, lines `:726-748`, with
   no write. `check_collection` keeps its stamp arm and becomes `match self.existing().await? { Matches => {}, Stamp => { ..unchanged.. } }`.
3. New public methods, placed after `connect_to`:

   ```rust
   /// Opens [`COLLECTION`] for reading only (MOD-92 G8): `Ok(None)` when it does not exist.
   /// Creates no collection, no payload index, and stamps nothing: an unstamped collection of the
   /// right width is searched as it is (what `connect` would have stamped). The width and embedder
   /// check is `connect`'s own.
   ///
   /// # Errors
   /// [`StoreError::Backend`] when Qdrant cannot be reached, and `embedder mismatch`
   /// ([`is_embedder_mismatch`]) for another width or another embedder's vectors.
   pub async fn open_existing(settings: &QdrantSettings, embedder: E) -> Result<Option<Self>, StoreError>

   /// As [`open_existing`](Self::open_existing), on a named collection (tests use a throwaway one).
   pub async fn open_existing_to(settings: &QdrantSettings, embedder: E, collection: &str) -> Result<Option<Self>, StoreError>
   ```

   `open_existing_to` does `Self::new`, then `collection_exists` (mapped with `backend("collection
   exists", e)`), then returns `None` when absent, else `store.existing().await?` (result ignored:
   `Matches` and `Stamp` both pass), then `Some(store)`.

**Validate T1:**
```bash
cargo test -p htui-store --all-features --test read_only_pg -- --test-threads=1
cargo test -p htui-store --all-features --test qdrant_live -- --test-threads=1   # HTUI_TEST_QDRANT_URL set in the sandbox
cargo test -p htui-store --all-features --test migrations -- --test-threads=1   # connect_headless refactor
cargo test -p htui-store --all-features --lib identity
cargo sqlx prepare --workspace --check   # must already pass: no SQL text changed
```

---

## T2: `WaitingRow.since` (htui-worker)

**Files:** `crates/htui-worker/src/views.rs`, `crates/htui/src/store_worker.rs`,
`crates/htui/src/ui/top_bar.rs`, `crates/htui/src/ui/overlay/waiting_list.rs`.

### Tests first (`views.rs` `mod tests`, next to the reason tests at `:1574-1845`)

- Each case reuses that reason's existing candidate builder: `escalation()`, `parked`, `step`,
  `rejected` and `permission`.
- Each case gives the rows **distinct** stamps:
  - `item.updated_at = demo_at(1, 0)`
  - `run.updated_at = demo_at(2, 5)`
  - step `updated_at = demo_at(2, 6)`
  - the judge's `updated_at = demo_at(2, 7)`
  - the permission is created at minute 30
- Each case asserts the row's `since` against a **literal** instant, not a helper, so that each
  reason pins its own source.

| Test | Asserts `since ==` |
|---|---|
| `a_gate_row_is_since_its_steps_last_change` | the parked step's `updated_at` |
| `a_judge_failed_row_is_since_the_judges_last_change` | the failed judge's `updated_at` |
| `a_selection_row_is_since_its_runs_last_change` | `run.updated_at` |
| `an_unblock_row_over_a_run_is_since_the_runs_last_change` | `run.updated_at`, for FollowRun and for Resume |
| `a_reopen_row_is_since_the_items_last_change` | `item.updated_at` |
| `an_interrupted_row_is_since_its_runs_last_change` | `run.updated_at`, with and without a rest step |
| `a_permission_row_is_since_the_request_was_opened` | `permission.created_at` |

The existing test helper `row(candidate, run, step, step_label, reason, text)` (`:1437`) keeps its
signature. It fills `since` through a new test-only `fn expected_since(candidate, run, step, reason)
-> DateTime<Utc>`, which applies the D-C rule by looking the run and step up in `candidate.runs`.
The seven cases above pin the rule independently. The permission `expected` closure (`:1923`) gains
`since: <the permission's created_at>`.

### `views.rs` changes

- `WaitingRow` (`:520`): add a field after `text`:

  ```rust
  /// When the row began waiting, as the store last stamped it (MOD-92 D5): the parked step's
  /// `updated_at` for a gate, the judge's for a failed judge, the run's for a selection, an
  /// interrupted run and an Unblock row over a run, the item's for a Reopen row, and the request's
  /// `created_at` for a permission. The store's clock (`set_updated_at()`'s `clock_timestamp()`); a
  /// later edit of that row moves it. The overlay does not show it; `htui waiting --json` does.
  pub since: DateTime<Utc>,
  ```

  `DateTime<Utc>` is `Eq`, so `WaitingView` and `TopBarState` keep `Eq`. `DateTime` is already
  imported (used by `RowKey`).
- `waiting()` (`:590`): the `row` closure (`:611`) gains a final parameter `since: DateTime<Utc>`
  and puts it into the `WaitingRow`. The call sites pass:
  - Gate (`:652`): `step.updated_at`
  - JudgeFailed (`:677`): `judge.updated_at`
  - Selection (`:685`): `run.updated_at`
  - Unblock (`:710`): `run.map_or(item.updated_at, |run| run.updated_at)`
  - Interrupted (`:741`): `run.updated_at`
  - Permission (`:765`): `since: p.permission.created_at`
- Add one sentence to the `waiting()` doc pointing at `WaitingRow::since`.

### htui literal fixes (tests only)

| File | Site | Change |
|---|---|---|
| `crates/htui/src/store_worker.rs` | `:3811` `vec![WaitingRow {` | add `since:` with the instant the test's data stamps. Read the test: if it compares against a view built by `htui_worker::waiting`, take the source row's `updated_at`; else `DateTime::UNIX_EPOCH`. |
| `crates/htui/src/ui/top_bar.rs` | `row()` `:86` | `since: chrono::DateTime::UNIX_EPOCH` |
| `crates/htui/src/ui/overlay/waiting_list.rs` | `gate` `:433`, `permission` `:445`, `reopen` `:453`, `second` `:616` | `since: DateTime::UNIX_EPOCH` |

The overlay does not render `since`, so no `.snap` changes. Confirm with a full `cargo insta test`
(memory: snapshot impact needs a full insta run).

### `store_worker.rs`: one visibility change for T3

`fn permissions_or_unknown` (`:2021`) becomes `pub(crate)`, and its doc gains "also `htui waiting
--json`'s rule (MOD-92)". T3 calls it and duplicates nothing. T2 owns the file, so T3 does not touch
it.

**Validate T2:**
```bash
cargo test -p htui-worker --lib views
cargo test -p htui --all-features --lib -- waiting top_bar store_worker   # A-6
cargo insta test -p htui --all-features --check   # no .snap moved
```

---

## T3: the verbs (htui)

**Files:** `crates/htui/src/cli.rs`, `crates/htui/src/state_cmd.rs` (new),
`crates/htui/src/concepts.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/main.rs`,
`crates/htui/tests/state_cli_pg.rs` (new), plus the new snapshot files:
- `crates/htui/src/snapshots/htui__state_cmd__tests__*.snap`
- `crates/htui/tests/snapshots/state_cli_pg__*.snap`

### `cli.rs`

**`Args`**: add after `limit` (`:100`):

```rust
/// With `--search-items`: print one versioned JSON document instead of lines
/// (`docs/htui-cli.md`).
#[arg(long, requires = "search_items", conflicts_with = "index_items")]
pub json: bool,

/// With `--search-items`: only the project whose repository, registered on this box, contains
/// PATH (the deepest one).
#[arg(long, value_name = "PATH", requires = "search_items", conflicts_with_all = ["index_items", "project"])]
pub cwd: Option<PathBuf>,
```

**`Command`** (`:105`): add after `Provision` and before `Mcp`:

```rust
/// Print the top bar's facts and exit: store state, workspace, box, working and waiting counts.
/// Reads only (MOD-92): the DSN comes from the OS keyring; nothing is written. `--json` prints one
/// versioned document (`docs/htui-cli.md`); exit 3 when the store does not answer, 2 on a refusal.
Status(StateArgs),
/// Print what waits on you (`R-TUI-11`): gates, failed judges, selections, Unblock rows, interrupted
/// runs and permission requests, oldest reason first as the overlay orders them, with their age.
/// Reads only, as `status`.
Waiting(StateArgs),
```

**New `StateArgs`**, placed after `WorkerArgs`:

```rust
/// `htui status` / `htui waiting`'s flags (MOD-92 D1). None reads the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, clap::Args)]
pub struct StateArgs {
    /// Print one versioned JSON document instead of text.
    #[arg(long)]
    pub json: bool,
    /// Scope to the project with this slug instead of the first workspace.
    #[arg(long, value_name = "SLUG", conflicts_with = "cwd")]
    pub project: Option<String>,
    /// Scope to the project whose repository, registered on this box, contains PATH.
    #[arg(long, value_name = "PATH")]
    pub cwd: Option<PathBuf>,
}
```

**Tests** (in `cli.rs` `mod tests`, written first):

| Test | Asserts |
|---|---|
| `status_and_waiting_parse_their_three_flags` | `htui status` gives `Command::Status(StateArgs::default())`. `htui waiting --json --project htui` and `htui status --cwd .` parse to the expected values. |
| `state_project_and_cwd_exclude_each_other` | `htui status --project a --cwd .` gives `Err`, and the same for `waiting`. |
| `state_verbs_refuse_tui_flags_beside_them` | `htui --demo status`, `htui --offline waiting`, and `htui status --decisions` (a top-level-only flag after the subcommand) give `Err`. `htui status --log x` gives `Ok` (global). |
| `json_and_cwd_need_search_items_at_top_level` | `htui --json`, `htui --cwd .`, `htui --index-items --json` and `htui --search-items q --cwd . --project p` give `Err`. `htui --search-items q --json --cwd .` gives `Ok`. |
| `no_state_argument_reads_the_environment` | Modelled on `no_worker_argument_reads_the_environment` (`:248`): for `status` and `waiting`, no argument has `get_env()` except the propagated `log`. `R-STO-1`. |

Also extend `the_concepts_flags_parse_and_exclude_each_other`'s bad list with `--json` alone.

### `state_cmd.rs` (new; `pub mod state_cmd;` in `lib.rs`, alphabetical, after `skills`)

**Module doc:**

> `htui status`, `htui waiting`, and the document `--search-items --json` prints (MOD-92,
> `R-TUI-11`, `R-ID-2`, `R-ID-6`, `R-STO-1`, `R-NF-2`). Read-only by construction: the session is
> `PgStore::connect_read_only` (Postgres refuses writes), the cache is never opened, and `box.toml` is
> read with `identity::load` under `config_root_path` (nothing is created). Every outcome is one
> document on stdout under `--json` and an exit code: 0 answered, 3 the store (or the index) did not
> answer, 2 refused, 1 failed. The schema is `docs/htui-cli.md`'s; `SCHEMA_VERSION` bumps only on a
> breaking change.

**Public API, in file order:**

```rust
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use htui_core::model::{BoxId, ItemId, ProjectId, ProjectRef, RunId, Scope, StepId, WorkspaceId, WorkspaceSummary};
use htui_core::store::{ReadStore as _, StoreError, WriteStore as _};
use htui_store::vector::Hit;
use htui_store::{HeadlessError, Identity, PgStore};
use htui_worker::{WaitingReason, WaitingView};
use serde::Serialize;

/// The schema version every document carries (MOD-92 D4).
pub const SCHEMA_VERSION: u32 = 1;

/// Which read-only verb runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb { Status, Waiting }

/// How a state verb ends; `main` maps it to the exit code and prints it on stderr.
#[derive(Debug)]
pub enum StateExit {
    /// Exit 3: Postgres (or, for a search, Qdrant) did not answer. Never a crash report.
    Offline(String),
    /// Exit 2: refused before any answer (no DSN, keyring, schema, scope, embedder mismatch). Never a crash report.
    Refused(String),
    /// Exit 1: a read failed after the connect with something other than `Unreachable`, the model
    /// could not load, or stdout failed (not a closed pipe). Reported to Sentry.
    Failed(String),
}
impl StateExit { #[must_use] pub const fn code(&self) -> u8 { /* 3, 2, 1 */ } }
impl core::fmt::Display for StateExit { /* the sentence only, as WorkerExit */ }
impl std::error::Error for StateExit {}

/// What `answer` needs that `run` reads from the keyring, `box.toml` and the clock: the test seam.
#[derive(Debug, Clone)]
pub struct Context<'a> {
    pub dsn: &'a str,
    /// `box.toml`'s identity; `None` when there is none (then `"box": null`, and `--cwd` is refused).
    pub identity: Option<&'a Identity>,
    pub now: DateTime<Utc>,
    pub connect_timeout: Duration,
}

/// `htui status` / `htui waiting`: keyring DSN (`secret::get_dsn`), `identity::load` over
/// `identity::config_root_path()`, `Utc::now()`, `pg::CONNECT_TIMEOUT`; then `answer`, then `emit`
/// to a locked stdout. A missing DSN or an unreadable keyring or `box.toml` is answered as a refusal
/// document without connecting.
///
/// # Errors
/// [`StateExit`]; its code is the exit status.
pub async fn run(verb: Verb, args: cli::StateArgs) -> Result<(), StateExit>

/// The answer to `verb` over `ctx`. Never fails: a failure is an [`Answer`] too.
pub async fn answer(verb: Verb, args: &cli::StateArgs, ctx: &Context<'_>) -> Answer

/// Writes `answer`: under `json` one compact document and a newline, else the human text; a reader
/// that closed the pipe is not an error (`write_out`). Then the exit: `Ok` for 0, else the
/// [`StateExit`] carrying the human sentence (`main` prints it on stderr; nothing here prints to stderr
/// but search's "no matches" / "the index is empty" notes).
///
/// # Errors
/// [`StateExit`] for exit 1-3; [`StateExit::Failed`] when stdout fails with anything but `BrokenPipe`.
pub fn emit(out: &mut dyn Write, answer: &Answer, json: bool) -> Result<(), StateExit>

/// The JSON document of `answer` (what `emit` writes under `--json`), for snapshots.
#[must_use]
pub fn document(answer: &Answer) -> serde_json::Value
```

**Answer model (public, so `concepts.rs` and the tests can build one):**

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Answer { pub kind: Kind, pub outcome: Outcome }

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome { Status(StatusBody), Waiting(WaitingBody), Search(SearchAnswer), Failed(Failure) }

/// A failure before an answer, with Postgres's state as `store` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure { pub store: StoreState, pub error: ErrorDoc }   // store is Offline | Refused | Error, never Online

/// What `--search-items` found: the query echo, then the index's state.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchAnswer { pub query: String, pub decisions: bool, pub index: IndexAnswer }

#[derive(Debug, Clone, PartialEq)]
pub enum IndexAnswer { Hits(Vec<Hit>), Empty, Offline(String), Refused(String), Failed(String) }

impl Answer {
    /// 0, 1, 2 or 3, by D3 and A-2.
    #[must_use] pub fn exit_code(&self) -> u8
}
impl Failure {
    /// `HeadlessError` -> Failure: `Store(Unreachable)` offline/`unreachable`; `MigrationsPending`
    /// refused/`migrations_pending`; `BelowTarget` refused/`below_target`; `Store(_)` refused/
    /// `store_refused` (A-8). `message` is `format!("{:#}", concepts::headless_refusal(err))`.
    #[must_use] pub fn from_connect(err: HeadlessError) -> Self
    /// A read after the connect: `Unreachable` offline/`unreachable`, else error/`read_failed`.
    #[must_use] pub fn from_read(err: &StoreError) -> Self
    /// A refusal with `code` and `message` (store `refused`).
    #[must_use] pub fn refused(code: ErrorCode, message: impl Into<String>) -> Self
}
```

**Serde document types (exact names; all `#[derive(Debug, Clone, PartialEq, Eq, Serialize)]`
unless they hold an `f32`):**

```rust
#[serde(rename_all = "snake_case")] pub enum Kind { Status, Waiting, Search }
#[serde(rename_all = "snake_case")] pub enum StoreState { Online, Offline, Refused, Error }
#[serde(rename_all = "snake_case")] pub enum IndexState { Online, Empty, Offline, Refused, Error }

#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Unreachable, NoDsn, KeyringUnreadable, BoxIdentity, MigrationsPending, BelowTarget, StoreRefused,
    UnknownProject, NoBox, CwdUnresolved, CwdUnmatched, CwdAmbiguous, ReadFailed,
    NoQdrantUrl, QdrantUrlUnusable, QdrantUnreachable, EmbedderMismatch, ModelUnavailable,
}

pub struct ErrorDoc { pub code: ErrorCode, pub message: String }

pub struct WorkspaceDoc { pub id: WorkspaceId, pub slug: String, pub name: String }
pub struct ProjectDoc  { pub id: ProjectId, pub slug: String, pub name: String }
pub struct BoxDoc      { pub id: BoxId, pub hostname: String, pub registered: bool }

pub struct StatusBody {
    pub workspace: Option<WorkspaceDoc>,          // null: the store holds no workspace
    pub projects: Vec<ProjectDoc>,                // the scope's, in scope order
    #[serde(rename = "box")]
    pub this_box: Option<BoxDoc>,                 // null: no box.toml on this machine
    pub working: usize,
    pub waiting: usize,
    pub permissions_known: bool,
}

pub struct WaitingBody {
    pub workspace: Option<WorkspaceDoc>,
    pub projects: Vec<ProjectDoc>,
    pub working: usize,
    pub permissions_known: bool,
    pub rows: Vec<WaitingRowDoc>,
}

#[serde(rename_all = "snake_case")]
pub enum ReasonDoc { Gate, JudgeFailed, Selection, Unblock, Interrupted, Permission }
// `impl From<WaitingReason> for ReasonDoc`: an exhaustive match, so a new reason fails to compile
// here rather than serialising silently.

pub struct WaitingRowDoc {
    pub reason: ReasonDoc,
    pub item_key: String,
    pub item_id: ItemId,
    pub run_id: Option<RunId>,
    pub step_id: Option<StepId>,
    pub step: Option<String>,        // WaitingRow.step_label; null when empty
    pub text: String,
    pub since: String,               // rfc3339(row.since), A-11
    pub age_seconds: u64,            // age_seconds(now, row.since), D-D
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchDoc {
    pub index: IndexState,
    pub query: String,
    pub decisions: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hits: Option<Vec<HitDoc>>,   // Some for online and empty only (A-2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorDoc>,     // Some for offline, refused, error
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HitDoc {
    pub key: String,
    pub place: &'static str,                     // PointType::as_str: "item" | "document" | "requirement"
    pub document_kind: Option<String>,           // hit.document's kind
    pub resolution: Option<&'static str>,        // Resolution::as_str
    pub requirement_state: Option<&'static str>, // RequirementState::as_str
    pub score: f32,                              // serde_json writes the shortest f32 text
    pub snippet: String,
}
```

**The envelope.** `document()` serialises
`Envelope { version: SCHEMA_VERSION, kind, store, #[serde(flatten)] body }`, with the body one of:
- `&StatusBody`
- `&WaitingBody`
- `&SearchDoc`, with `store: Online`
- a private `FailureBody { error: &ErrorDoc }`, with `store` taken from the `Failure`

So field order is `version, kind, store`, then the body in declaration order. serde_json writes struct
fields in order.

**Private helpers, all pure and unit-tested:**
- `fn rfc3339(at: DateTime<Utc>) -> String`: `SecondsFormat::Secs`, `use_z = true`.
- `fn age_seconds(now: DateTime<Utc>, since: DateTime<Utc>) -> u64`:
  `u64::try_from((now - since).num_seconds()).unwrap_or(0)`.
- `fn age_text(seconds: u64) -> String`: `42s`, `5m`, `3h`, `2d`, in human output only.
- `struct Target<'a> { workspace: Option<&'a WorkspaceSummary>, projects: Vec<&'a ProjectRef>, scope: Scope }`.
  - `fn first_workspace(workspaces: &[WorkspaceSummary]) -> Target<'_>` (plan G4).
    `Scope::from_workspace(first)`, or an empty scope with `WorkspaceId::default()` when there is
    none.
  - `fn project_target<'a>(workspaces: &'a [WorkspaceSummary], project: ProjectId) -> Option<Target<'a>>`
    (G7). The first workspace in `workspaces()` order that lists the project. The scope is
    `Scope { workspace_id, project_ids: vec![project] }`.
  - The `--project` slug is resolved with `ws.projects.iter().find(|p| p.slug == slug)`. If no
    workspace has it: `Failure::refused(UnknownProject, "no project with slug `{slug}`")`, the
    sentence `concepts::scopes` uses.
- `fn project_for_cwd(cwd: &Path, checkouts: &[(ProjectId, PathBuf)]) -> CwdMatch`, with private
  `enum CwdMatch { None, One(ProjectId), Ambiguous(Vec<ProjectId>) }`. Pure; both sides are
  already canonical (see the next section).
- `fn canonical_checkouts(rows: Vec<(ProjectId, String)>) -> Vec<(ProjectId, PathBuf)>`.
  `std::fs::canonicalize` on each stored path; a row that does not canonicalise is skipped.
- `fn status_body(target: &Target<'_>, view: &WaitingView, this_box: Option<BoxDoc>) -> StatusBody`.
- `fn waiting_body(target: &Target<'_>, view: &WaitingView, now: DateTime<Utc>) -> WaitingBody`.
- `fn human_status(body: &StatusBody) -> String`: the top bar's line, `"{workspace|no workspace} ·
  {hostname|no box} · online · {working} working · {waiting} waiting"`. Same separator and
  placeholders as `ui/top_bar.rs:30-62`.
- `fn human_waiting(body: &WaitingBody) -> String`: one line per row,
  `"{key:<10} {reason:<12} {step:<16} {age:>4}  {text}"`. `reason` is `WaitingReason::label`.
  `key`, `step` and `text` have control characters stripped, with `format_hit`'s `clean` rule (a gate
  note must not drive the terminal). An empty list writes nothing to stdout.

**`answer` data flow (status and waiting):**
1. `PgStore::connect_read_only(ctx.dsn, ctx.identity.map(|i| i.box_id), ctx.connect_timeout)`.
   `Err(e)` gives `Outcome::Failed(Failure::from_connect(e))`.
2. `pg.workspaces()`. A failure (any read from here on) gives `Failure::from_read`.
3. The target:
   - `args.project`: `project_target` by slug.
   - `args.cwd`: the `--cwd` resolution below.
   - otherwise: `first_workspace`.
4. These reads mirror `serve_waiting` (`store_worker.rs:2041`). They run sequentially on the
   2-connection pool:
   - `pg.active_runs(&scope)`
   - `ReadStore::waiting_candidates(&pg, &scope)`
   - `crate::store_worker::permissions_or_unknown(WriteStore::open_permissions(&pg, &scope).await)`.
     `Unreachable` propagates as offline; any other error gives `None`, which is
     `permissions_known: false`.
5. `let view = htui_worker::waiting(&scope, active, &candidates, permissions.as_deref());`, the
   TUI's own classifier.
6. Status only, `this_box`:
   - `ctx.identity` is `None`: `None`.
   - Otherwise `pg.box_info()`, which reads `this_box`:
     - `Some(info)`: `BoxDoc { id, hostname: info.hostname, registered: true }`, the hostname the
       top bar shows (`app/update.rs:434`).
     - `None`: `BoxDoc { id, hostname: identity.hostname, registered: false }`.
7. Build the body with `ctx.now`.

**`--cwd` resolution (inside `answer`, also used by search):**
- **The input path.** `std::fs::canonicalize(arg)`. A relative path resolves against the process's
  working directory, and `--cwd .` works. Symlinks are resolved; on Windows the result is the
  `\\?\` verbatim form with on-disk casing.
  - If canonicalising fails: `Failure::refused(CwdUnresolved, "`{arg}` cannot be resolved on this
    box")`. The sentence names the path as typed, never its target, by the `root_path` rule
    (`R-BOX-4`).
  - If `ctx.identity` is `None`: `refused(NoBox, "no box.toml on this box: start htui once so this box
    has an identity")`.
- **The checkouts.** `pg.repo_paths(box_id)`, mapped to projects. For each distinct project of every
  workspace, `WriteStore::repos(project)` builds `RepoId → ProjectId`. A `repo_box_path` whose repo
  belongs to no workspace's project is ignored. Then `canonical_checkouts`.
- **Why canonicalise both sides.** Stored paths are already canonical (`hierarchy.rs:405`
  `canonical(path)` → `htui_core::root_path::canonical_root`, which stores `\\?\` "as returned" on
  Windows). Canonicalising them again at match time puts both sides through the same
  `std::fs::canonicalize` on the same machine, with these effects:
  - The prefix form is identical: `\\?\C:\` on both sides, and `\\?\UNC\` for mapped or `subst`
    drives.
  - Casing is the file system's on both sides (`GetFinalPathNameByHandleW`), so Windows' case
    insensitivity needs no case folding.
  - A checkout reached through a symlink matches its target.
  - A stored path that no longer exists cannot contain an existing `cwd`, so skipping it loses
    nothing.
- **The match.** `project_for_cwd` keeps every checkout `c` with `cwd.starts_with(c)`. That is
  component-wise, so `/src/htui2` is not inside `/src/htui`.
  - It picks the greatest `c.components().count()`, the deepest repo, which is right for a nested
    checkout.
  - At that depth: one project gives `One`; two or more distinct projects give `Ambiguous` (A-9).
    Two repos of the *same* project at the same path are `One`.
  - `None` gives `refused(CwdUnmatched, "`{arg}` is in no repository registered on this box")`.
  - `Ambiguous(ps)` gives `refused(CwdAmbiguous, "... registered for projects {slugs}")`.
  - `One(p)` gives `project_target(workspaces, p)`.

**Tests** (unit, in `state_cmd.rs` `mod tests`; fixed `Uuid::from_u128` ids and `demo_at`
instants, so no filters are needed):

| Test | Asserts |
|---|---|
| `status_document_pins_every_field` | insta snapshot `status_online`: a workspace, two projects, a registered box, 1 working, 2 waiting, `permissions_known: true`. |
| `status_document_without_workspace_or_box` | Snapshot `status_no_workspace_no_box`: `"workspace": null`, `"projects": []`, `"box": null`, zeros. |
| `waiting_document_has_one_row_per_reason` | Snapshot `waiting_online`: six rows, one per `WaitingReason`, each with its `reason` text (`judge_failed` snake case), `step: null` for the Reopen row, `since` in whole-second `Z` form, `age_seconds` from a fixed `now`. |
| `reason_doc_covers_every_reason` | Every `WaitingReason` maps, and `serde_json::to_value(ReasonDoc::from(r))` equals `r.label().replace(' ', "_")`. |
| `age_is_floored_at_zero` | `since` after `now` gives 0. 93 s gives 93. |
| `failure_documents_carry_store_and_code` | Snapshots `offline_unreachable` (status), `refused_no_dsn` (waiting) and `error_read_failed` (status): only `version, kind, store, error`. |
| `search_documents_by_index_state` | Snapshots `search_online` (two `Hit`s built as in `concepts.rs` `tests::hit`, one a closed item's `prd` document), `search_index_empty` (`hits: []`), `search_index_offline` (no `hits`, `error.code = qdrant_unreachable`) and `search_index_refused`. |
| `exit_codes_follow_d3` | `Answer::exit_code`: online 0, offline 3, refused 2, error 1; search empty 0, index offline 3, refused 2, error 1. |
| `connect_failures_map_to_codes` | `Failure::from_connect` on each `HeadlessError` shape gives (store, code) per A-8. `Unreachable` → offline. A `Backend` connect error → refused/`store_refused`. `message` equals `format!("{:#}", concepts::headless_refusal(..))`. |
| `emit_writes_one_parsable_line_and_survives_a_closed_pipe` | Under `json`, the bytes are one line ending in `\n` and parse back to `document(&answer)`. With a `BrokenPipe` writer `emit` returns the answer's exit, never `Failed`; with an `Other` writer, `Failed`. Reuse the `Failing` writer shape from `lib.rs` tests. |
| `human_status_is_the_top_bar_line` | Snapshot `status_line`. |
| `human_waiting_strips_control_characters` | A gate note with `\x1b[2J` prints without the escape. |
| `cwd_picks_the_deepest_checkout` | `/src/mono` (project A) and `/src/mono/sub` (B): a cwd under `sub` gives B; under `mono/other` gives A; `/src/mono2` gives `None`. |
| `cwd_tie_between_projects_is_ambiguous` | Same path for A and B gives `Ambiguous([A, B])`. Same path twice for A gives `One(A)`. |
| `cwd_through_a_symlink_matches_its_target` | `#[cfg(unix)]`, tempdir. A symlink to the checkout dir, canonicalised, gives `One`. |
| `cwd_windows_verbatim_paths_compare` | `#[cfg(windows)]`, tempdir. A stored path from `canonicalize` (`\\?\C:\...`) matches a cwd typed as `C:\...` with different casing after `canonicalize`. Compiled on the Windows cross-check. |

**`tests/state_cli_pg.rs`** (new):
- Header: `#![cfg(feature = "testkit")]`, the `requirements_pg.rs` module-doc idiom (each case
  prints `testkit::SKIP` and returns without `HTUI_TEST_DATABASE_URL`, and panics when `CI` is set),
  and `use htui_store::testkit;`.
- Every case runs over `testkit::demo_db()`, which loads fixed ids and fixed `updated_at`s (the
  trigger is BEFORE UPDATE only).
- The fixture identity is `Identity { box_id: ids::BOX, hostname: "DESKTOP-HTUI".into() }`
  (`testkit::fixture_box_store`'s).
- Each case calls `state_cmd::answer(verb, &args, &Context { dsn: &db.url, identity, now, connect_timeout })`,
  then `emit` into a `Vec<u8>`.
- `now` is a fixed instant, but the gate seeding re-stamps rows by the trigger. So the snapshots use
  `insta::with_settings!` filters:

  ```rust
  const UUID: (&str, &str) = (r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "[uuid]");
  const STAMP: (&str, &str) = (r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", "[timestamp]");
  const AGE: (&str, &str) = (r#""age_seconds": \d+"#, r#""age_seconds": "[age]""#);
  const MESSAGE: (&str, &str) = (r#""message": "[^"]*""#, r#""message": "[message]""#);
  ```

  Fixed demo ids may stay unfiltered where no random id appears. Use `UUID` only in the cases that
  insert rows.
- `seed_gate(&db)`: raw `sqlx::query` on `db.pool` sets demo `RUN_2` to `awaiting_approval` and
  `STEP_R2_PRD` to `awaiting_approval`.

| Test | Asserts |
|---|---|
| `status_defaults_to_the_first_workspace` | The workspace is the first by name (`Graphics` sorts before `Platform`; `workspaces()` orders by `w.name`). Exit 0. `box.registered == true`, with hostname from the row. Snapshot `status_default_workspace`. |
| `waiting_matches_the_tuis_classifier` | `seed_gate`, `--project htui`. The rows (reason, item, run, step, `since`) equal `htui_worker::waiting` computed over the **read-write** `db.store`'s three reads for the same scope (parity with `serve_waiting`). The gate row's `since` equals `run_step.updated_at` read back raw. Snapshot `waiting_project_htui`. |
| `cwd_scopes_to_the_registered_checkout` | Through `db.store` (read-write): `create_repo` on `PROJECT_HTUI` and `upsert_repo_box_path` with the canonical tempdir for `ids::BOX`. `--cwd <tempdir>/a/b` (created) scopes to `htui`. A sibling tempdir gives refused/2 `cwd_unmatched`. With `identity: None`, refused/2 `no_box`. |
| `an_unknown_project_is_refused` | `--project nope` gives `store: refused`, `error.code: unknown_project`, exit 2, and `emit` returns `StateExit::Refused`. Snapshot `unknown_project`. |
| `an_unreachable_store_is_offline` | `postgres://nobody:nothing@127.0.0.1:1/none` with a 2 s timeout gives offline/3 for both verbs. Needs no database, never skips. Snapshot `offline` (`MESSAGE` filter). |
| `a_pending_schema_is_refused` | Over `testkit::bare_db()`: refused/2 `migrations_pending`. |
| `the_verbs_write_nothing` | Counts `box`, `app_user`, `item`, `run`, `run_step` and `app_setting` (with `updated_at` maxima) before and after one `status` and one `waiting` with an **unregistered** box id. All are equal. |
| `the_read_only_session_is_what_answers` | Proves `answer` connects read-only, not through `connect_headless`: after a `status` with a fresh `BoxId::new()`, no `box` row has that id. |

### `concepts.rs`

1. `SearchOptions` (`:38`): add `pub cwd: Option<PathBuf>` and `pub json: bool`, with docs. It stays
   `Eq`.
2. Extract the DSN sentence into `pub(crate) const NO_DSN: &str = "no Postgres DSN is stored; run
   `htui --set-dsn` first";`. `open()` (`:96`) and `state_cmd::run` both use it.
3. `scopes` (`:157`) becomes `pub(crate)`.
4. `open()` stays for `--index-items` only, which writes the index by design. Its doc says so.
5. `search_items` (`:421`) changes signature:

   ```rust
   /// `--search-items`: the best hits, as lines or (`--json`) one document (MOD-92). Read-only:
   /// `PgStore::connect_read_only` and `QdrantStore::open_existing`; a missing collection is an empty
   /// index, never created. The embedding model may still be downloaded into the user cache (plan D8);
   /// its notes go to stderr.
   ///
   /// # Errors
   /// [`StateExit`](crate::state_cmd::StateExit): 3 when Postgres or Qdrant does not answer, 2 on a
   /// refusal (no URL, no DSN, schema, scope, embedder mismatch), 1 on a model or read failure.
   pub async fn search_items(options: &SearchOptions) -> Result<(), crate::state_cmd::StateExit>
   ```

   **Order:**
   1. `settings_from(get_qdrant_url, get_qdrant_api_key)`. Failures are refused: `no_qdrant_url`,
      `keyring_unreadable` or `qdrant_url_unusable`.
   2. `secret::get_dsn()`: `no_dsn` or `keyring_unreadable`.
   3. Only when `options.cwd` is set, `identity::load(&identity::config_root_path()?)` (A-4).
   4. `PgStore::connect_read_only(dsn, box_id, CONNECT_TIMEOUT)`, mapped with `Failure::from_connect`.
   5. Projects:
      - `cwd`: the shared `--cwd` resolution gives a project, mapped to its slug, then
        `scopes(&pg, Some(slug))`.
      - otherwise `scopes(&pg, options.project.as_deref())`, today's default of every workspace (D6).
   6. `model::ensure_model_noting(|l| eprintln!("{l}"))` and `RtenEmbedder::load` on
      `spawn_blocking`, as `open()` does. A failure gives `IndexAnswer::Failed`.
   7. `QdrantStore::open_existing(&settings, embedder)`:
      - `Ok(None)`: `Empty`.
      - `Err(e)` with `is_embedder_mismatch(&e)`: `Refused(format!("{:#}", qdrant_refusal(e)))`.
      - any other `Err`: `Offline(format!("{:#}", qdrant_refusal(e)))`.
   8. `store.search(&query(..))`: `Hits`, or on `Err` the same mismatch/offline split.
   9. `state_cmd::emit(&mut std::io::stdout().lock(), &answer, options.json)`.

   **Human mode:** `Hits` prints `format_hit` lines on stdout. An empty `Hits` prints today's
   stderr "no matches (run `htui --index-items` if the index is empty)". `Empty` prints "the index
   is empty; run `htui --index-items`" on stderr. Both exit 0.
6. Tests in `concepts.rs`: `search_options_default_keeps_every_workspace`, a pure check that
   `cwd: None, project: None` builds an all-workspace query through `query`. The document
   snapshots live in `state_cmd`.

### `lib.rs`

- `pub mod state_cmd;` (alphabetical, after `skills`).
- **Dispatch order in `run`** (`:95`):
  1. `Mcp`
  2. `Worker`
  3. `init_tracing`
  4. `Provision` (`:109`)
  5. **new:**

     ```rust
     if let Some(cli::Command::Status(state)) = args.command.clone() {
         return state_cmd::run(state_cmd::Verb::Status, state).await.map_err(anyhow::Error::from);
     }
     if let Some(cli::Command::Waiting(state)) = args.command.clone() {
         return state_cmd::run(state_cmd::Verb::Waiting, state).await.map_err(anyhow::Error::from);
     }
     ```

  6. `set_dsn`, `clear_dsn`, `index_items`
  7. `search_items`, which now passes `cwd: args.cwd, json: args.json` and `.map_err(anyhow::Error::from)`
  8. `keys::resolve` (`:137`)

  The new verbs and the search branch both run after `init_tracing` and before `keys::resolve`.
  Update the `run` doc's startup-order paragraph.
- **Stdout is clean on these paths:**
  - `Mcp` and `Worker` return before this point.
  - `init_tracing` installs nothing without `--log`. With it, the subscriber, the `sentry_tracing`
    layer included, writes to the file.
  - No `println!` exists in htui-store, htui-core or htui-worker non-test code. The only hits are
    `runtime.rs` test skips.
  - Qdrant's compatibility check, which prints to stdout, stays skipped (`vector.rs:669`).
  - Model notes go to `eprintln!`.
  - Sentry is initialised in `main::body` and prints nothing.
  - `keys::resolve` and its `notices` `eprintln!` come after this dispatch.
- **Extract the BrokenPipe rule.**

  ```rust
  /// Writes `bytes` and flushes; a reader that closed the pipe early (`| head`) is not a failure
  /// (MOD-67 M2 B-4), any other write error is.
  pub(crate) fn write_out(out: &mut dyn std::io::Write, bytes: &[u8]) -> std::io::Result<()>
  ```

  `print_keys` becomes `write_out(out, text.as_bytes()).map_err(Into::into)`. Its test is unchanged
  and still green. `state_cmd::emit` uses `write_out`.

### `main.rs`

- `exit_code` (`:102`): add
  `.or_else(|| error.downcast_ref::<htui::state_cmd::StateExit>().map(htui::state_cmd::StateExit::code))`
  before `.unwrap_or(1)`. Update its doc: "a `StateExit`'s (MOD-92: 3, 2 or 1)".
- `reports_to_sentry` (`:132`): add `&& !matches!(error.downcast_ref::<htui::state_cmd::StateExit>(),
  Some(htui::state_cmd::StateExit::Offline(_) | htui::state_cmd::StateExit::Refused(_)))`. Exit 2
  is already excluded by `code != 2`. This adds the exit-3 case **by type** (an unreachable store is
  a state, not a crash), and the doc names MOD-92.
- The `main` doc's exit-code paragraph adds "3 for a state verb whose store or index did not answer
  (MOD-92)".
- **Tests:**
  - `state_exits_are_3_2_1_and_only_failures_reach_sentry`: `code()` values; `exit_code`
    downcasts, wrapped too; Offline and Refused never reach Sentry; Failed does.
  - Extend `only_mcp_skips_the_teardown_grace` with `htui status` → `SHUTDOWN`.

**Validate T3:**
```bash
cargo test -p htui --all-features --lib -- state_cmd cli concepts tests::print_keys
cargo test -p htui --all-features --bin htui
cargo test -p htui --all-features --test state_cli_pg -- --test-threads=1
cargo test -p htui --all-features --test concepts_search
cargo insta test -p htui --all-features --check
```

---

## T4: documentation

**Files:** `docs/htui-cli.md` (new) and `README.md`.

- **`docs/htui-cli.md`:**
  - The three verbs and their flags.
  - The envelope (D4) and every document, copied from T3's snapshots, with every field typed
    (`string`, `uuid`, `RFC 3339 UTC, whole seconds, Z`, `integer`, `null when ...`).
  - The `reason` and `index` vocabularies, and the `error.code` vocabulary with the exit code each
    one ends in.
  - The D3 table with A-2's search rows.
  - The versioning rule: a removed, renamed or retyped field bumps `version`; an added field or a
    new enum value does not, so consumers must ignore unknown fields and treat unknown enum values
    as "other".
  - The no-document cases (A-1).
  - "No writes" and how it is enforced:
    - `default_transaction_read_only`;
    - no bootstrap;
    - `identity::load` under `config_root_path`;
    - `open_existing`;
    - the cache is never opened;
    - the model cache exception (D8);
    - `--log` writes only the file asked for.
  - The `since` caveat (D-C: the row's last change on the store's clock) and the clock-skew note
    (D-D).
  - Scope rules: the first workspace by name, `--project`, `--cwd` (the deepest registered
    checkout; canonical paths; `cwd_ambiguous`; needs `box.toml`). Search defaults to every
    workspace.
- **`README.md`:**
  - In "Command-line options" (`:87`), add rows for `status [--json] [--project SLUG | --cwd PATH]`,
    `waiting [...]`, `--json` and `--cwd` (with `--search-items`).
  - In the `--project` row, also mention `status`/`waiting`'s own `--project`.
  - Add a short "Scripting" subsection (or a paragraph under Search) that links `docs/htui-cli.md`
    and gives the exit codes.
  - Add a line in "Further reading".
  - Update "Contents" if a new `##` heading is added.
- **Validate:** `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`. Check the
  examples by hand against the snapshots.

---

## Files

### Files to create
| File | Purpose | Task |
|---|---|---|
| `crates/htui-store/tests/read_only_pg.rs` | read-only session refuses writes, registers nothing, every CLI read works | T1 |
| `crates/htui/src/state_cmd.rs` | verbs, scope and `--cwd`, documents, human text, `StateExit` | T3 |
| `crates/htui/tests/state_cli_pg.rs` | end-to-end over `demo_db`, insta snapshots | T3 |
| `crates/htui/src/snapshots/htui__state_cmd__tests__*.snap` | document shapes | T3 |
| `crates/htui/tests/snapshots/state_cli_pg__*.snap` | live shapes | T3 |
| `docs/htui-cli.md` | schema, exit codes, versioning | T4 |

### Files to modify
| File | Changes | Task |
|---|---|---|
| `crates/htui-store/src/pg/mod.rs` | `Session`, `open_pool(.., session)`, `headless_checks`, `connect_read_only` | T1 |
| `crates/htui-store/src/identity.rs` | `read_box_toml`, `load` | T1 |
| `crates/htui-store/src/vector.rs` | `new`, `existing`, `open_existing`, `open_existing_to` | T1 |
| `crates/htui-store/tests/qdrant_live.rs` | five `open_existing` cases (A-5) | T1 |
| `crates/htui-worker/src/views.rs` | `WaitingRow.since`, set in `waiting()`, seven tests | T2 |
| `crates/htui/src/store_worker.rs` | `since` in the `:3811` literal; `permissions_or_unknown` `pub(crate)` | T2 |
| `crates/htui/src/ui/top_bar.rs` | test literal | T2 |
| `crates/htui/src/ui/overlay/waiting_list.rs` | four test literals | T2 |
| `crates/htui/src/cli.rs` | `Status`, `Waiting`, `StateArgs`, `--json`, `--cwd`; tests | T3 |
| `crates/htui/src/concepts.rs` | `SearchOptions.{cwd,json}`, `NO_DSN`, `scopes` `pub(crate)`, read-only `search_items` | T3 |
| `crates/htui/src/lib.rs` | `pub mod state_cmd`, dispatch, `write_out` | T3 |
| `crates/htui/src/main.rs` | `exit_code`, `reports_to_sentry`, docs, tests | T3 |
| `README.md` | options rows, scripting pointer | T4 |

### File-set check
- **T1 ∩ T2 = ∅.**
  - T1 = {`htui-store/src/pg/mod.rs`, `htui-store/src/identity.rs`, `htui-store/src/vector.rs`,
    `htui-store/tests/read_only_pg.rs`, `htui-store/tests/qdrant_live.rs`}.
  - T2 = {`htui-worker/src/views.rs`, `htui/src/store_worker.rs`, `htui/src/ui/top_bar.rs`,
    `htui/src/ui/overlay/waiting_list.rs`}.
  - No shared `.sqlx`, `.snap`, `Cargo.toml` or `Cargo.lock` change.
  - **Hidden coupling:** T2's `cargo test -p htui` compiles htui-store. On a *shared* tree it builds
    T1's half-done state. Run the two in separate worktrees, or have T2 validate `-p htui-worker`
    first and `-p htui` after T1 has committed.
- **T3's files:** `htui/src/cli.rs`, `htui/src/state_cmd.rs`, `htui/src/concepts.rs`,
  `htui/src/lib.rs`, `htui/src/main.rs`, `htui/tests/state_cli_pg.rs`, and the two snapshot globs
  above. T3 does not touch T1's or T2's files. It *reads* `WaitingRow.since`,
  `permissions_or_unknown`, `connect_read_only`, `identity::load` and `open_existing`.

---

## Data flow

1. `main` → `htui::run`.
2. `init_tracing` (file only, or none).
3. `state_cmd::run(verb, args)`:
   1. `secret::get_dsn()` and `identity::load(config_root_path)` (no create), each mapped to a
      refusal if needed.
   2. `answer`:
      1. `PgStore::connect_read_only` (session `default_transaction_read_only=on`; headless checks;
         no bootstrap).
      2. `workspaces()`.
      3. Target: first, `--project` or `--cwd`. For `--cwd`: `repo_paths(box)` + `repos(project)`,
         canonicalise, longest prefix.
      4. `active_runs`, `waiting_candidates`, `open_permissions` (via `permissions_or_unknown`).
      5. `htui_worker::waiting`, then the `StatusBody` / `WaitingBody` (`since` → `rfc3339`,
         `age_seconds` against `now`).
   3. `emit` (one JSON line or human text, BrokenPipe-safe).
   4. `Err(StateExit)` for codes 1-3.
4. `main::exit_code` (downcast) and `reports_to_sentry` (Offline and Refused are excluded by type).
5. `--search-items` follows the same path through `concepts::search_items`, with
   `QdrantStore::open_existing` (no create, no index, no stamp) and an `index` state on the document.

## Build sequence
1. T1 and T2 in parallel (separate worktrees), tests first in each. Commit incrementally.
2. Merge both, then run T1's and T2's validates on the merged tree.
3. T3: `cli.rs` tests and types, then the `state_cmd` pure parts with their unit snapshots, then
   `answer`/`run`, then `concepts::search_items`, then `lib.rs`/`main.rs`, then `state_cli_pg.rs`.
4. T4 from T3's accepted snapshots.
5. The gate: fmt, clippy ×2 (with `--all-features` and featureless), `cargo test --workspace
   --all-features -- --test-threads=1`, `cargo sqlx prepare --workspace --check` (expected no-op),
   and the workflow-docs validator.

## Risks the plan does not list
| Risk | Mitigation |
|---|---|
| A connection pooler in transaction mode (PgBouncer) may reject the `options` startup parameter, which would make every verb `store_refused` | Document it in `docs/htui-cli.md`. Fallback if reported: `PgPoolOptions::after_connect` running `SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY`. Not built now. |
| A `box.toml` copied from another machine: read-only cannot detect it (detection registers), so `--cwd` would read the other box's `repo_paths` | Documented on `connect_read_only`. Starting the TUI once re-mints. Status still reports `registered`. |
| MOD-93 polls every N seconds; a persistent exit-1 failure reaches Sentry on every poll | Plan D3 keeps exit 1 reported. Recommendation for the maintainer: also exclude `StateExit::Failed` by type if GlitchTip volume becomes noise. |
| A hung keyring prompt (secret-service) blocks the verb indefinitely | Same as today's `--search-items`. MOD-93 bounds its process call. Not addressed here. |
| `--search-items --json` on a box without the model downloads ~134 MB even when the collection is missing | Today's verb does the same. Plan D8 accepts the model cache. The notes go to stderr. |
